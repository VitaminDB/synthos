//! Worker-thread IO для `.syn` пакетов: open, save, create.
//!
//! `Bundle` (mmap-backed) живёт в process-wide реестре `bundle_registry`
//! по пути файла. UI получает доступ через `with_bundle(&path, f)` без
//! блокировки render-thread'а на чтении.
//!
//! Все длинные операции (`Bundle::open` ~10 ms, `BundleEditor::commit` ~10 ms
//! + WAL/fsync, `BundleBuilder::write` секунды) выполняются в `std::thread::spawn`.
//! Результат пушится обратно в UI через RwSignal-`set` (сигналы thread-safe
//! через `Mutex` внутри).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use syngui::{tr, trn};
use synaptix_bundle::inspect;
use synaptix_bundle::pack_plan::PackPlan;
use synaptix_bundle::{
    Bundle, BundleEditor, BundleMeta, ChunkStatus, ChunkType, ProgressEvent,
};

use crate::context::AppCtx;

use super::state::{
    install_dirty_tracker, BundleStats, ComponentLayers, CreateProgress, FileEntryView, LoadState,
    OpenBundle, PendingOp, SynExplorerCtx,
};
use super::tree_build;

/// Реестр открытых bundle'ов по абсолютному пути. `Arc<Bundle>` shared между
/// preview-операциями (`read_file`) и таб-обзорщиками; mmap дроп'ится автоматом
/// при `remove`.
fn bundle_registry() -> &'static Mutex<HashMap<PathBuf, Arc<Bundle>>> {
    static R: OnceLock<Mutex<HashMap<PathBuf, Arc<Bundle>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Получить `Arc<Bundle>` для пути, если он открыт. Без блокировки UI:
/// одна Mutex-захват + clone Arc.
pub fn get_bundle(path: &Path) -> Option<Arc<Bundle>> {
    bundle_registry()
        .lock()
        .ok()
        .and_then(|g| g.get(path).cloned())
}

/// Заменить bundle в реестре (предыдущий mmap дроп'ается). Использется
/// после успешного open/save.
fn install_bundle(path: PathBuf, bundle: Bundle) -> Arc<Bundle> {
    let arc = Arc::new(bundle);
    if let Ok(mut g) = bundle_registry().lock() {
        g.insert(path, arc.clone());
    }
    arc
}

/// Дроп bundle из реестра. Вызывается при close_bundle.
pub fn drop_bundle(path: &Path) {
    if let Ok(mut g) = bundle_registry().lock() {
        g.remove(path);
    }
}

/// Прочитать payload файла из открытого пакета. None — bundle не открыт
/// или такого файла нет. Преобразует `Cow<[u8]>` в owned `Arc<Vec<u8>>` для
/// кеширования в `OpenBundle.preview_cache`.
pub fn read_file_owned(path: &Path, name: &str) -> Option<Arc<Vec<u8>>> {
    let bundle = get_bundle(path)?;
    match bundle.read_file(name) {
        Ok(cow) => Some(Arc::new(cow.into_owned())),
        Err(e) => {
            eprintln!("[syn-explorer] read_file `{name}` failed: {e}");
            None
        }
    }
}

/// Снимок данных пакета для построения `OpenBundle` (выполняется в worker'е).
struct BundleSnapshot {
    meta: BundleMeta,
    files: Vec<FileEntryView>,
    stats: BundleStats,
    layers: Vec<ComponentLayers>,
    dir_tree: Vec<syngui::widgets::TreeNode>,
}

/// Собрать snapshot из открытого Bundle. Никаких сигналов — функция
/// thread-safe и вызывается в worker'е.
fn snapshot(bundle: &Bundle) -> BundleSnapshot {
    let meta = bundle.meta().clone();
    let stats = compute_stats(bundle);
    let files = collect_files(bundle);
    let layers = collect_layers(bundle);
    let dir_tree = tree_build::build_dir_tree(bundle);
    BundleSnapshot { meta, files, stats, layers, dir_tree }
}

/// Разобрать состав каждого `tensors:*`-чанка. Читается только
/// safetensors-заголовок внутри уже отображённого mmap — это миллисекунды
/// даже на 77-гигабайтном бандле, поэтому делается сразу при открытии, а не
/// лениво по клику на вкладку.
fn collect_layers(bundle: &Bundle) -> Vec<ComponentLayers> {
    let meta = bundle.meta();
    let names: Vec<String> = bundle
        .cdir()
        .entries
        .iter()
        .filter(|e| e.is_alive() && matches!(e.kind_typed(), ChunkType::Tensors))
        .map(|e| e.name.trim_start_matches("tensors:").to_string())
        .collect();

    let mut out = Vec::with_capacity(names.len());
    for component in names {
        let Ok(slice) = bundle.tensors_slice_named(&component) else {
            continue;
        };
        let Ok(tensors) = inspect::read_header_slice(slice) else {
            continue;
        };
        // Подсказка для нераспознанных имён: у однокомпонентного
        // `acestep_vae.syn` чанк зовётся `main`, и роль читается только из
        // id/purpose самого бандла.
        let hint = format!("{component} {} {}", meta.purpose, meta.id);
        let mut by_role: Vec<_> = inspect::bytes_by_role(&tensors, Some(&hint))
            .into_iter()
            .collect();
        by_role.sort_by(|a, b| b.1.dense.cmp(&a.1.dense));
        out.push(ComponentLayers {
            tensor_count: tensors.len(),
            bytes: tensors.iter().map(|t| t.bytes).sum(),
            groups: inspect::group_tensors(&tensors, Some(&hint)),
            by_role,
            component,
        });
    }
    out
}

fn compute_stats(bundle: &Bundle) -> BundleStats {
    let mut s = BundleStats {
        total_size: bundle.size(),
        format_version: bundle.version(),
        ..Default::default()
    };
    for e in &bundle.cdir().entries {
        let alive = matches!(e.status_typed(), ChunkStatus::Alive);
        if alive {
            s.alive_chunks += 1;
        } else {
            s.tombstoned_chunks += 1;
            continue;
        }
        match e.kind_typed() {
            ChunkType::Tensors => s.tensor_chunks += 1,
            ChunkType::QuantizedTensors => s.quantized_chunks += 1,
            ChunkType::File => s.file_chunks += 1,
            ChunkType::Ref => s.ref_chunks += 1,
            _ => {}
        }
    }
    s
}

fn collect_files(bundle: &Bundle) -> Vec<FileEntryView> {
    bundle
        .cdir()
        .entries
        .iter()
        .map(|e| FileEntryView {
            name: e.name.clone(),
            kind: e.kind_typed(),
            size: e.payload_len,
            crc32c: e.crc32c,
            sha256_short: e.sha256.as_ref().map(|h| short_hex(h, 8)),
            tag: e.tag,
            alive: matches!(e.status_typed(), ChunkStatus::Alive),
        })
        .collect()
}

fn short_hex(bytes: &[u8], n: usize) -> String {
    let take = n.min(bytes.len());
    let mut s = String::with_capacity(take * 2);
    for b in &bytes[..take] {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Открыть пакет в worker-thread. Результат — `OpenBundle` или ошибка
/// (Error-диалог). По завершении выставляет `ctx.active_bundle`,
/// `load_state = Idle`, инсталлирует dirty-tracker.
///
/// Все обращения к сигналам/контексту делегируются обратно в main-thread
/// через `run_on_main_thread` — signal-runtime и context-provider оба
/// `thread_local!`, в worker'е их нет.
pub fn open_async(ctx: SynExplorerCtx, path: PathBuf) {
    ctx.load_state.set(LoadState::Loading);

    std::thread::spawn(move || {
        let result = Bundle::open(&path).map(|b| {
            let snap = snapshot(&b);
            (b, snap)
        });
        match result {
            Ok((bundle, snap)) => {
                install_bundle(path.clone(), bundle);
                run_on_main_thread(move || {
                    let open_bundle = OpenBundle::new(
                        path,
                        snap.meta,
                        snap.files,
                        snap.stats,
                        snap.layers,
                        snap.dir_tree,
                    );
                    // dirty-tracker — create_effect требует thread_local
                    // runtime, поэтому только на main thread.
                    install_dirty_tracker(open_bundle);
                    ctx.active_bundle.set_always(Some(open_bundle));
                    ctx.load_state.set(LoadState::Idle);
                    notify_info_main(tr!("explorer.notify.bundle_opened"));
                });
            }
            Err(e) => {
                let msg = e.to_string();
                run_on_main_thread(move || {
                    ctx.load_state.set(LoadState::Idle);
                    ctx.show_error(tr!("explorer.error.open_failed.title"), msg);
                });
            }
        }
    });
}

/// Перезагрузить активный пакет с диска (drop registry + open снова).
/// Сбрасывает pending_ops и meta до значений с диска — несохранённые
/// правки теряются (caller должен проверить `dirty` перед вызовом).
pub fn reload_async(ctx: SynExplorerCtx) {
    let Some(active) = ctx.active_untracked() else {
        return;
    };
    let path = active.path.get_untracked();
    drop_bundle(&path);
    // Сбрасываем active_bundle перед re-open — UI получит "пустое" состояние
    // → loading-overlay → готовое состояние.
    ctx.active_bundle.set_always(None);
    open_async(ctx, path);
}

/// Закрыть активный пакет: дроп из registry, очистить active_bundle.
/// При наличии dirty caller должен заранее показать `ConfirmCloseUnsaved`.
pub fn close_active(ctx: SynExplorerCtx) {
    if let Some(active) = ctx.active_untracked() {
        let path = active.path.get_untracked();
        drop_bundle(&path);
    }
    ctx.active_bundle.set_always(None);
    ctx.current_tab
        .set(super::state::TabKind::Overview);
}

/// Применить pending_ops + изменения meta к bundle через `BundleEditor` и
/// `commit`. На успех — reload bundle (свежий mmap + сигналы). На ошибку —
/// Error-диалог; pending_ops остаются — пользователь может попробовать снова.
pub fn save_async(ctx: SynExplorerCtx) {
    let Some(active) = ctx.active_untracked() else {
        return;
    };
    if !active.dirty.get_untracked() {
        notify_info_main(tr!("explorer.notify.no_unsaved_changes"));
        return;
    }
    let path = active.path.get_untracked();
    let pending = active.pending_ops.get_untracked();
    let meta = active.meta.get_untracked();
    let original = active.original_meta.get_untracked();

    ctx.load_state.set(LoadState::Saving);

    let meta_changed = match (serde_json::to_vec(&meta), serde_json::to_vec(&original)) {
        (Ok(a), Ok(b)) => a != b,
        _ => true,
    };

    std::thread::spawn(move || {
        let outcome = apply_edits(&path, pending, if meta_changed { Some(meta) } else { None });
        match outcome {
            Ok(()) => {
                drop_bundle(&path);
                match Bundle::open(&path) {
                    Ok(b) => {
                        let snap = snapshot(&b);
                        install_bundle(path.clone(), b);
                        run_on_main_thread(move || {
                            // Перенести свежие данные в существующий
                            // OpenBundle handle через signals (main-thread).
                            active.meta.set_always(snap.meta.clone());
                            active.original_meta.set_always(snap.meta);
                            active.files.update(|v| *v = snap.files);
                            active.stats.set(snap.stats);
                            active.layers.update(|v| *v = snap.layers);
                            active.dir_tree.update(|v| *v = snap.dir_tree);
                            active.pending_ops.update(|v| v.clear());
                            active.preview_cache.update(|m| m.clear());
                            active.reload_gen.update(|n| *n = n.wrapping_add(1));
                            ctx.load_state.set(LoadState::Idle);
                            notify_info_main(tr!("explorer.notify.saved"));
                        });
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        run_on_main_thread(move || {
                            ctx.load_state.set(LoadState::Idle);
                            ctx.show_error(tr!("explorer.error.reload_after_save.title"), msg);
                        });
                    }
                }
            }
            Err(msg) => {
                run_on_main_thread(move || {
                    ctx.load_state.set(LoadState::Idle);
                    ctx.show_error(tr!("explorer.error.save_failed.title"), msg);
                });
            }
        }
    });
}

fn apply_edits(
    path: &Path,
    ops: Vec<PendingOp>,
    new_meta: Option<BundleMeta>,
) -> Result<(), String> {
    let mut editor = BundleEditor::open(path).map_err(|e| e.to_string())?;
    if let Some(meta) = new_meta {
        editor.set_meta(meta);
    }
    for op in ops {
        match op {
            PendingOp::AddFile { name, data, tag } => {
                // `Arc::try_unwrap` чтобы избежать лишнего copy — в pending_ops
                // данные шарятся ради cheap-clone; если refcount == 1,
                // забираем Vec<u8> владением.
                let bytes = Arc::try_unwrap(data).unwrap_or_else(|a| (*a).clone());
                editor
                    .add_file(&name, bytes, tag)
                    .map_err(|e| format!("add_file `{name}`: {e}"))?;
            }
            PendingOp::RemoveFile { name } => {
                editor
                    .remove_file(&name)
                    .map_err(|e| format!("remove_file `{name}`: {e}"))?;
            }
            PendingOp::Rename { old, new } => {
                editor
                    .rename(&old, &new)
                    .map_err(|e| format!("rename `{old}` → `{new}`: {e}"))?;
            }
        }
    }
    editor.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// Упаковать модель по готовому плану. Worker-thread: `PackPlan::into_builder`
/// → `write`. На успех открывает получившийся пакет. Прогресс идёт в
/// `ctx.create_progress` через throttled-callback (см. `make_progress_cb`).
///
/// Валидация плана и заполнение метаданных остаются на стороне UI
/// (`PackWizard::validation_error`) — сюда приходит уже согласованный план.
pub struct PackOptions {
    pub delete_sources: bool,
    pub sha256: bool,
    pub blake3: bool,
    pub cdir_json: bool,
    /// Что квантовать при упаковке. Пусто — обычная побайтовая упаковка.
    pub quant: super::quant_pack::QuantDecision,
}

pub fn create_from_plan_async(
    ctx: SynExplorerCtx,
    plan: PackPlan,
    out: PathBuf,
    opts: PackOptions,
) {
    // Место проверяем до старта: узнать про «no space left» на последнем
    // байте после десяти минут упаковки — худший из возможных вариантов.
    let target_dir = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let required = plan.required_space();
    let avail = synaptix_bundle::available_space(&target_dir).unwrap_or(u64::MAX);
    if avail < required {
        ctx.show_error(
            tr!("explorer.error.insufficient_space.title"),
            tr!(
                "explorer.error.insufficient_space.detail",
                path = target_dir.display(),
                avail = gib(avail),
                required = gib(required),
                total = gib(plan.payload_bytes()),
                max_comp = gib(plan.max_component_bytes()),
            ),
        );
        return;
    }

    let progress_handle = ctx.create_progress.get_untracked();
    let progress_gen = ctx.create_progress_gen;
    if let Ok(mut g) = progress_handle.lock() {
        *g = CreateProgress::default();
    }
    progress_gen.set(0);
    ctx.load_state.set(LoadState::Creating);

    std::thread::spawn(move || {
        let cb = make_progress_cb(progress_handle.clone(), progress_gen);
        let res = build_with_quant(plan, &opts)
            .map(|b| {
                let mut b = b
                    .with_progress(cb)
                    .with_delete_sources_after_pack(opts.delete_sources);
                if opts.sha256 {
                    b = b.with_sha256(true);
                }
                if opts.blake3 {
                    b = b.with_blake3(true);
                }
                if opts.cdir_json {
                    b = b.cdir_format(synaptix_bundle::CdirFormat::Json);
                }
                b
            })
            .and_then(|b| b.write(&out).map_err(|e| e.to_string()));
        match res {
            Ok(()) => {
                run_on_main_thread(move || {
                    ctx.load_state.set(LoadState::Idle);
                    // Диалог показывал прогресс; оставлять его поверх
                    // открывающегося пакета было бы странно.
                    ctx.close_dialog();
                    // Папка получила новый `.syn` — список слева обновляем,
                    // иначе собранного пакета в нём не будет до перезахода.
                    super::bookmarks::refresh_selected(ctx);
                    open_async(ctx, out);
                });
            }
            Err(e) => {
                run_on_main_thread(move || {
                    ctx.load_state.set(LoadState::Idle);
                    ctx.show_error(tr!("explorer.error.create_failed.title"), e);
                });
            }
        }
    });
}

/// Собрать билдер: обычный план либо, если выбрано квантование, с подменой
/// главного компонента на квантующий поток.
///
/// Квантуется только главный компонент — тот, чьи слои разобраны в мастере
/// и по которому посчитана оценка размера. Остальные компоненты копируются
/// как есть, иначе обещанный в UI размер разошёлся бы с настоящим.
fn build_with_quant(
    plan: PackPlan,
    opts: &PackOptions,
) -> Result<synaptix_bundle::BundleBuilder, String> {
    use super::quant_pack;

    if opts.quant.is_empty() {
        return plan.into_builder().map_err(|e| e.to_string());
    }
    let Some(main_idx) = plan.components.iter().position(|c| c.enabled) else {
        return plan.into_builder().map_err(|e| e.to_string());
    };

    let device = synaptix_core::device::Device::Cuda(0);
    let decision = opts.quant.clone();
    let decide = move |name: &str, shape: &[usize]| decision.for_tensor(name, shape);
    let mut stream = Some(quant_pack::QuantizingStream::new(
        &plan.components[main_idx].paths,
        &decide,
        device,
    )?);
    let quantized = stream.as_ref().map(|s| s.quantized_count()).unwrap_or(0);
    if quantized == 0 {
        // Ни один тензор не подошёл — незачем городить поток и объявлять
        // возможность формата, которой в бандле нет.
        return plan.into_builder().map_err(|e| e.to_string());
    }
    let manifest = stream
        .as_ref()
        .ok_or_else(|| "квантующий поток потерян".to_string())?
        .manifest_json()?;

    let mut b = synaptix_bundle::BundleBuilder::new(&plan.meta.id, &plan.meta.version);
    if !plan.meta.arch.is_empty() {
        b = b.arch(&plan.meta.arch);
    }
    if !plan.meta.purpose.is_empty() {
        b = b.purpose(&plan.meta.purpose);
    }
    for (i, c) in plan.components.iter().enumerate().filter(|(_, c)| c.enabled) {
        let prefix = (!c.prefix.is_empty()).then_some(c.prefix.as_str());
        if let Some(p) = prefix {
            b = b.component(&c.name, p);
        }
        if i == main_idx {
            b = b.add_tensor_stream(&c.name, quant_pack::boxed(stream_take(&mut stream)?));
            continue;
        }
        b = b.add_safetensors_component(&c.name, c.paths.clone(), prefix);
    }
    for f in plan.aux.iter().filter(|f| f.enabled) {
        b = b.add_file_path(&f.rel, &f.path, f.tag).map_err(|e| e.to_string())?;
    }
    b = b
        .add_file_bytes(
            quant_pack::MANIFEST_NAME,
            manifest,
            synaptix_bundle::FileTag::Inference,
        )
        .map_err(|e| e.to_string())?;
    // Читатель без поддержки раскладки обязан отказаться открывать бандл,
    // а не искать тензоры, которых больше нет под прежними именами.
    b = b.require_capability(quant_pack::CAP_QUANT);
    tracing::info!(target: "syn-explorer", tensors = quantized, "упаковка с квантованием");
    Ok(b)
}

/// Забрать поток из `Option` с внятной ошибкой вместо `unwrap`.
fn stream_take(
    slot: &mut Option<super::quant_pack::QuantizingStream>,
) -> Result<super::quant_pack::QuantizingStream, String> {
    slot.take()
        .ok_or_else(|| "квантующий поток уже израсходован".to_string())
}

fn gib(n: u64) -> String {
    format!("{:.2}", n as f64 / (1024.0 * 1024.0 * 1024.0))
}

/// Сборка throttled-callback'а: payload-данные пишутся в `Arc<Mutex>`, а
/// `progress_gen` поднимается ≈раз в 60 мс или на ключевых событиях
/// (Plan/ItemStart/ItemDone/Finalizing/Done). UI подписан на `progress_gen`
/// через `Reactive`, поэтому request_redraw делается с разумной частотой.
fn make_progress_cb(
    handle: Arc<Mutex<CreateProgress>>,
    gen_signal: syngui::prelude::RwSignal<u64>,
) -> synaptix_bundle::ProgressCallback {
    let last_emit = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1)));
    // Локальный generation-счётчик: signal-runtime thread_local, читать сигнал
    // из worker'а нельзя (`get_untracked` упадёт — slots пусты). `set_always`
    // маршалит запись на main thread.
    let gen_counter = Arc::new(AtomicU64::new(0));
    Arc::new(move |ev| {
        let mut force = false;
        if let Ok(mut g) = handle.lock() {
            match ev {
                ProgressEvent::Plan { total_bytes, total_items, payload_bytes } => {
                    g.bytes_done = 0;
                    g.bytes_total = total_bytes;
                    g.payload_total = payload_bytes;
                    g.items_total = total_items;
                    g.items_done = 0;
                    g.finalizing = false;
                    g.current_item = String::new();
                    g.stage_label = trn!("explorer.progress.preparing", total_items);
                    force = true;
                }
                ProgressEvent::ItemStart { index, name, bytes: _ } => {
                    g.current_item = name.clone();
                    g.items_done = index + 1;
                    g.stage_label = tr!(
                        "explorer.progress.compressing",
                        done = g.items_done, total = g.items_total, name = name
                    );
                    force = true;
                }
                ProgressEvent::Bytes { delta } => {
                    g.bytes_done = g.bytes_done.saturating_add(delta);
                }
                ProgressEvent::ItemDone { index: _, name: _, deleted_sources: _ } => {
                    force = true;
                }
                ProgressEvent::Finalizing => {
                    g.finalizing = true;
                    g.stage_label = tr!("explorer.progress.finalizing");
                    force = true;
                }
                ProgressEvent::Done => {
                    g.bytes_done = g.bytes_total;
                    g.stage_label = tr!("explorer.progress.done");
                    force = true;
                }
            }
        }
        let should_emit = force
            || last_emit
                .lock()
                .ok()
                .map(|t| t.elapsed() >= Duration::from_millis(60))
                .unwrap_or(true);
        if should_emit {
            if let Ok(mut t) = last_emit.lock() {
                *t = Instant::now();
            }
            // Counter живёт в worker'е, чтобы не читать сигнал (его runtime
            // — thread_local на main). `set_always` маршалит запись на main.
            let next = gen_counter.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
            gen_signal.set_always(next);
        }
    })
}

/// Показать info-уведомление через глобальный `AppCtx::notifications`. ВЫЗЫВАТЬ
/// ТОЛЬКО ИЗ main-thread (`use_context` thread_local). Из worker'ов —
/// обернуть в `run_on_main_thread`.
fn notify_info_main(msg: impl Into<String>) {
    let app = use_context::<AppCtx>();
    app.notifications.info(msg.into());
}
