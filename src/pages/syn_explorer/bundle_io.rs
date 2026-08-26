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
use synaptix_bundle::{
    Bundle, BundleBuilder, BundleEditor, BundleMeta, ChunkStatus, ChunkType, FileTag,
    ProgressEvent,
};

use crate::context::AppCtx;

use super::state::{
    install_dirty_tracker, BundleStats, CreateProgress, FileEntryView, LoadState, NewPackageForm,
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
    dir_tree: Vec<syngui::widgets::TreeNode>,
}

/// Собрать snapshot из открытого Bundle. Никаких сигналов — функция
/// thread-safe и вызывается в worker'е.
fn snapshot(bundle: &Bundle) -> BundleSnapshot {
    let meta = bundle.meta().clone();
    let stats = compute_stats(bundle);
    let files = collect_files(bundle);
    let dir_tree = tree_build::build_dir_tree(bundle);
    BundleSnapshot { meta, files, stats, dir_tree }
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

/// Создать новый пакет из формы NewPackage. Worker-thread:
/// `BundleBuilder::new(...).arch(...).purpose(...).add_safetensors_component(...)*.write(out)`.
/// На успех автоматически открывает созданный пакет в UI. Проброс прогресса в
/// `ctx.create_progress` идёт через throttled-callback (см. `make_progress_cb`).
pub fn create_async(ctx: SynExplorerCtx, form: NewPackageForm) {
    let id = form.id.get_untracked().trim().to_string();
    let version = form.version.get_untracked().trim().to_string();
    let arch = form.arch.get_untracked().trim().to_string();
    let purpose = form.purpose.get_untracked().trim().to_string();
    let out = form.out_path.get_untracked();
    let delete_sources = form.delete_sources.get_untracked();

    if id.is_empty() {
        ctx.show_error(tr!("explorer.error.missing_data.title"), tr!("explorer.error.missing_data.id_required"));
        return;
    }
    if version.is_empty() {
        ctx.show_error(tr!("explorer.error.missing_data.title"), tr!("explorer.error.missing_data.version_required"));
        return;
    }
    let Some(out) = out else {
        ctx.show_error(tr!("explorer.error.missing_data.title"), tr!("explorer.error.missing_data.out_path_required"));
        return;
    };

    // Снимем все компоненты в worker-friendly виде (plain owned values).
    let components_raw = form.components.get_untracked();
    if components_raw.is_empty() {
        ctx.show_error(
            tr!("explorer.error.missing_data.title"),
            tr!("explorer.error.missing_data.no_components"),
        );
        return;
    }
    let mut components: Vec<(String, PathBuf, Option<String>)> = Vec::new();
    for c in components_raw {
        let name = c.name.get_untracked().trim().to_string();
        let dir = c.source_dir.get_untracked();
        let prefix = c.prefix.get_untracked().trim().to_string();
        if name.is_empty() {
            ctx.show_error(
                tr!("explorer.error.missing_data.title"),
                tr!("explorer.error.missing_data.component_name_required"),
            );
            return;
        }
        let Some(dir) = dir else {
            ctx.show_error(
                tr!("explorer.error.missing_data.title"),
                tr!("explorer.error.missing_data.component_dir_required", name = name),
            );
            return;
        };
        components.push((name, dir, if prefix.is_empty() { None } else { Some(prefix) }));
    }

    // Проверка свободного места выполняется на старте — до того как открыли
    // tmp файл. Это даёт fail-fast: пользователь не ждёт 10 минут, чтобы
    // получить «no space left on device» на финальном байте.
    let target_dir = out
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    if let Some(err) = preflight_check_space(&components, &target_dir) {
        ctx.show_error(tr!("explorer.error.insufficient_space.title"), err);
        return;
    }

    let progress_handle = ctx.create_progress.get_untracked();
    let progress_gen = ctx.create_progress_gen;
    // Reset прогресс перед стартом, чтобы UI не показал старые цифры
    // в момент LoadState::Creating.
    if let Ok(mut g) = progress_handle.lock() {
        *g = CreateProgress::default();
    }
    progress_gen.set(0);
    ctx.load_state.set(LoadState::Creating);

    std::thread::spawn(move || {
        let cb = make_progress_cb(progress_handle.clone(), progress_gen);
        let res = build_bundle(
            &id,
            &version,
            &arch,
            &purpose,
            &components,
            &out,
            delete_sources,
            cb,
        );
        match res {
            Ok(()) => {
                run_on_main_thread(move || {
                    ctx.load_state.set(LoadState::Idle);
                    // Закрываем диалог NewPackage — он отображал прогресс-зону,
                    // и оставлять его поверх открываемого пакета было бы странно.
                    ctx.close_dialog();
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

/// Расчёт места на диске. Возвращает `Some(error_message)` если места не
/// хватает, `None` если всё ок. Эвристика: нам нужно `total + max_component`
/// байт на разделе `out` (tmp бандла + крупнейший tensors_stage tmp) + 64 МБ
/// запаса на cdir/паддинги.
fn preflight_check_space(
    components: &[(String, PathBuf, Option<String>)],
    target_dir: &Path,
) -> Option<String> {
    let mut total: u64 = 0;
    let mut max_comp: u64 = 0;
    for (_, dir, _) in components {
        // Считаем все .safetensors-шарды в директории, плюс остальные файлы
        // (которые попадут в File-чанки). Это не идеально совпадает с тем,
        // что увидит BundleBuilder, но даёт честную верхнюю границу.
        let mut comp_tensor_bytes: u64 = 0;
        let mut comp_aux_bytes: u64 = 0;
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for ent in rd.flatten() {
            let p = ent.path();
            if !p.is_file() {
                continue;
            }
            let size = match std::fs::metadata(&p) {
                Ok(m) => m.len(),
                Err(_) => continue,
            };
            let is_tensor = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.eq_ignore_ascii_case("safetensors"))
                .unwrap_or(false);
            if is_tensor {
                comp_tensor_bytes = comp_tensor_bytes.saturating_add(size);
            } else {
                comp_aux_bytes = comp_aux_bytes.saturating_add(size);
            }
        }
        total = total.saturating_add(comp_tensor_bytes).saturating_add(comp_aux_bytes);
        if comp_tensor_bytes > max_comp {
            max_comp = comp_tensor_bytes;
        }
    }
    // Запас на cdir/паддинги/чанк-хедеры. Cdir сам редко больше 1 МБ;
    // 64 МБ — безопасный round-up.
    let required = total
        .saturating_add(max_comp)
        .saturating_add(64 * 1024 * 1024);
    let avail = synaptix_bundle::available_space(target_dir).unwrap_or(u64::MAX);
    if avail < required {
        return Some(tr!(
            "explorer.error.insufficient_space.detail",
            path = target_dir.display(),
            avail = format!("{:.2}", avail as f64 / (1024.0 * 1024.0 * 1024.0)),
            required = format!("{:.2}", required as f64 / (1024.0 * 1024.0 * 1024.0)),
            total = format!("{:.2}", total as f64 / (1024.0 * 1024.0 * 1024.0)),
            max_comp = format!("{:.2}", max_comp as f64 / (1024.0 * 1024.0 * 1024.0)),
        ));
    }
    None
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

#[allow(clippy::too_many_arguments)]
fn build_bundle(
    id: &str,
    version: &str,
    arch: &str,
    purpose: &str,
    components: &[(String, PathBuf, Option<String>)],
    out: &Path,
    delete_sources: bool,
    progress: synaptix_bundle::ProgressCallback,
) -> Result<(), String> {
    let mut builder = BundleBuilder::new(id, version);
    if !arch.is_empty() {
        builder = builder.arch(arch);
    }
    if !purpose.is_empty() {
        builder = builder.purpose(purpose);
    }
    // Каждый компонент → отдельный `tensors:<name>` чанк. Внутри папки —
    // model.safetensors / index.json / glob (через `resolve_safetensors_in_dir`).
    // Aux-файлы (tokenizer.json, config.json, README.md) добавляем как File
    // только из *первой* папки — если у пользователя multi-component, эти
    // файлы обычно лежат рядом с одним из компонентов и дублировать их
    // нельзя (имена в bundle root конфликтуют).
    for (idx, (name, dir, prefix)) in components.iter().enumerate() {
        let paths = synaptix_bundle::resolve_safetensors_in_dir(dir)
            .map_err(|e| format!("{}: {e}", tr!("explorer.error.component_prefix", name = name)))?;
        builder = builder.add_safetensors_component(name, paths, prefix.as_deref());
        // Aux-файлы первого компонента.
        if idx == 0 {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for ent in rd.flatten() {
                    let p = ent.path();
                    if !p.is_file() {
                        continue;
                    }
                    let ext = p
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|s| s.to_ascii_lowercase())
                        .unwrap_or_default();
                    if ext == "safetensors" {
                        continue;
                    }
                    let aux_name = match p.file_name().and_then(|n| n.to_str()) {
                        Some(n) => n.to_string(),
                        None => continue,
                    };
                    builder = builder
                        .add_file_path(&aux_name, &p, FileTag::Inference)
                        .map_err(|e| format!("add_file `{aux_name}`: {e}"))?;
                }
            }
        }
    }
    builder = builder
        .with_progress(progress)
        .with_delete_sources_after_pack(delete_sources);
    builder
        .write(out)
        .map_err(|e| format!("write `{}`: {e}", out.display()))?;
    Ok(())
}

/// Показать info-уведомление через глобальный `AppCtx::notifications`. ВЫЗЫВАТЬ
/// ТОЛЬКО ИЗ main-thread (`use_context` thread_local). Из worker'ов —
/// обернуть в `run_on_main_thread`.
fn notify_info_main(msg: impl Into<String>) {
    let app = use_context::<AppCtx>();
    app.notifications.info(msg.into());
}
