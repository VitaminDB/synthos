//! Высокоуровневые действия страницы — обёртки над `bundle_io` + `bookmarks`
//! с интеграцией rfd (native file dialogs). Все функции не падают: каждая
//! ошибка превращается в Error-диалог или notification.
//!
//! rfd работает асинхронно (futures::Future) — на синхронном UI-callback'е мы
//! проще запускаем blocking-вариант в отдельном потоке. Это не блокирует
//! event-loop, потому что rfd на Linux использует xdg-portal (D-Bus,
//! ожидание пользователя — естественно блокирующее в worker'е).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use syngui::tr;
use synaptix_bundle::FileTag;

use super::bookmarks;
use super::bundle_io;
use super::state::{
    ComponentLayers, DialogKind, OpenBundle, PackWizard, PendingOp, SynExplorerCtx, TabKind,
};

/// Открыть .syn через native file dialog. Воркер запускает rfd blocking, по
/// результату вызывает `bundle_io::open_async` (на main-thread).
pub fn pick_and_open_bundle(ctx: SynExplorerCtx) {
    std::thread::spawn(move || {
        let path = rfd::FileDialog::new()
            .add_filter("Syn bundle", &["syn"])
            .set_title(tr!("explorer.dialog.rfd.open_title"))
            .pick_file();
        if let Some(p) = path {
            run_on_main_thread(move || bundle_io::open_async(ctx, p));
        }
    });
}

/// Открыть конкретный .syn (вызывается из карточки в left_panel).
pub fn open_bundle(ctx: SynExplorerCtx, path: PathBuf) {
    bundle_io::open_async(ctx, path);
}

/// Закрыть активный пакет с проверкой dirty. Если есть несохранённые правки —
/// показывает `ConfirmCloseUnsaved`; иначе сразу `bundle_io::close_active`.
pub fn close_active_bundle(ctx: SynExplorerCtx) {
    let Some(active) = ctx.active_untracked() else {
        return;
    };
    if active.dirty.get_untracked() {
        ctx.open_dialog(DialogKind::ConfirmCloseUnsaved);
    } else {
        bundle_io::close_active(ctx);
    }
}

/// Подтверждённое закрытие без сохранения — вызывается из ConfirmCloseUnsaved
/// диалога.
pub fn force_close_active_bundle(ctx: SynExplorerCtx) {
    bundle_io::close_active(ctx);
}

/// Применить pending_ops и meta-изменения. См. `bundle_io::save_async`.
pub fn save_active(ctx: SynExplorerCtx) {
    bundle_io::save_async(ctx);
}

/// Перезагрузить активный пакет с диска (drop mmap + open снова). Если есть
/// dirty — спрашиваем подтверждение (ConfirmCloseUnsaved семантически
/// подходит: тоже теряем правки).
pub fn reload_active(ctx: SynExplorerCtx) {
    let Some(active) = ctx.active_untracked() else {
        return;
    };
    if active.dirty.get_untracked() {
        ctx.open_dialog(DialogKind::ConfirmCloseUnsaved);
        return;
    }
    bundle_io::reload_async(ctx);
}

/// Подтверждённый force-reload (для случая ConfirmCloseUnsaved).
pub fn force_reload_active(ctx: SynExplorerCtx) {
    bundle_io::reload_async(ctx);
}

/// Выбрать папку для закладки и добавить её. Воркер, потом UI-thread пушит в
/// сигналы.
pub fn pick_and_add_bookmark(ctx: SynExplorerCtx) {
    std::thread::spawn(move || {
        let path = rfd::FileDialog::new()
            .set_title(tr!("explorer.dialog.rfd.add_bookmark_title"))
            .pick_folder();
        if let Some(p) = path {
            bookmarks::add_bookmark(ctx, p);
        }
    });
}

/// Импортировать произвольный файл в активный пакет (как FileTag::Inference)
/// — кладёт в pending_ops, не пишет на диск. Сохранение — кнопкой Save.
pub fn pick_and_import_file(ctx: SynExplorerCtx) {
    let Some(active) = ctx.active_untracked() else {
        ctx.show_error(tr!("explorer.error.no_bundle.title"), tr!("explorer.error.no_bundle.message"));
        return;
    };
    std::thread::spawn(move || {
        let picked = rfd::FileDialog::new()
            .set_title(tr!("explorer.dialog.rfd.import_title"))
            .pick_file();
        let Some(path) = picked else {
            return;
        };
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        if name.is_empty() {
            run_on_main_thread(move || {
                ctx.show_error(tr!("explorer.error.import.title"), tr!("explorer.error.import.no_name"));
            });
            return;
        }
        match std::fs::read(&path) {
            Ok(data) => {
                let data = Arc::new(data);
                run_on_main_thread(move || {
                    active.pending_ops.update(|v| {
                        v.push(PendingOp::AddFile {
                            name,
                            data,
                            tag: FileTag::Inference,
                        });
                    });
                });
            }
            Err(e) => {
                let msg = e.to_string();
                run_on_main_thread(move || {
                    ctx.show_error(tr!("explorer.error.import.title"), msg);
                });
            }
        }
    });
}

/// Извлечь выбранный файл (selected_path) из пакета на диск. Save-as через rfd.
pub fn pick_and_extract_file(ctx: SynExplorerCtx) {
    let Some(active) = ctx.active_untracked() else {
        return;
    };
    let Some(name) = active.selected_path.get_untracked() else {
        ctx.show_error(
            tr!("explorer.error.extract.title"),
            tr!("explorer.error.extract.no_selection"),
        );
        return;
    };
    let path = active.path.get_untracked();
    let leaf = name
        .rsplit('/')
        .next()
        .unwrap_or(&name)
        .to_string();
    std::thread::spawn(move || {
        let save_to = rfd::FileDialog::new()
            .set_title(tr!("explorer.dialog.rfd.save_as_title", name = leaf))
            .set_file_name(&leaf)
            .save_file();
        let Some(out) = save_to else {
            return;
        };
        match bundle_io::read_file_owned(&path, &name) {
            Some(bytes) => {
                if let Err(e) = std::fs::write(&out, bytes.as_ref()) {
                    let msg = e.to_string();
                    run_on_main_thread(move || {
                        ctx.show_error(tr!("explorer.error.extract.title"), msg);
                    });
                }
            }
            None => {
                run_on_main_thread(move || {
                    ctx.show_error(
                        tr!("explorer.error.extract.title"),
                        tr!("explorer.preview.read_failed", name = name),
                    );
                });
            }
        }
    });
}

/// Удалить выбранный файл из пакета (поставить в pending_ops). UI должен
/// сначала спросить подтверждение через ConfirmDeleteFile.
pub fn request_delete_selected(ctx: SynExplorerCtx) {
    let Some(active) = ctx.active_untracked() else {
        return;
    };
    let Some(name) = active.selected_path.get_untracked() else {
        ctx.show_error(tr!("explorer.error.delete.title"), tr!("explorer.error.delete.no_selection"));
        return;
    };
    ctx.open_dialog(DialogKind::ConfirmDeleteFile { name });
}

/// Подтверждённое удаление — добавляет PendingOp::RemoveFile.
pub fn confirm_delete_file(active: OpenBundle, name: String) {
    active.pending_ops.update(|v| {
        v.push(PendingOp::RemoveFile { name: name.clone() });
    });
    // Если выбранный файл был именно удалён — сбрасываем selection.
    if active.selected_path.get_untracked().as_deref() == Some(name.as_str()) {
        active.selected_path.set(None);
    }
}

/// Упаковать конкретную модель: разбираем источник в worker'е и показываем
/// быструю карточку подтверждения. Это и есть «в один клик» — дальше
/// пользователю остаётся нажать «Собрать».
pub fn pack_source(ctx: SynExplorerCtx, path: PathBuf) {
    ctx.reset_wizard();
    ctx.wizard.scanning.set(true);
    ctx.open_dialog(DialogKind::PackConfirm);
    scan_into_wizard(ctx, path);
}

/// Открыть мастер без выбранного источника (кнопка «+» в шапке).
pub fn open_pack_wizard(ctx: SynExplorerCtx) {
    ctx.reset_wizard();
    ctx.open_dialog(DialogKind::PackWizard);
}

/// Перейти из быстрой карточки в мастер, сохранив разобранный план.
pub fn switch_to_wizard(ctx: SynExplorerCtx) {
    ctx.wizard.step.set(0);
    ctx.open_dialog(DialogKind::PackWizard);
}

/// Выбрать источник вручную: каталог модели или одиночный `.safetensors`.
/// Два отдельных действия вместо одного «выбрать» — нативные диалоги не
/// умеют предлагать файл и папку одновременно.
pub fn pick_source_dir(ctx: SynExplorerCtx) {
    std::thread::spawn(move || {
        let Some(p) = rfd::FileDialog::new()
            .set_title(tr!("explorer.dialog.rfd.pick_source_dir"))
            .pick_folder()
        else {
            return;
        };
        run_on_main_thread(move || scan_into_wizard(ctx, p));
    });
}

pub fn pick_source_file(ctx: SynExplorerCtx) {
    std::thread::spawn(move || {
        let Some(p) = rfd::FileDialog::new()
            .add_filter("safetensors", &["safetensors"])
            .set_title(tr!("explorer.dialog.rfd.pick_source_file"))
            .pick_file()
        else {
            return;
        };
        run_on_main_thread(move || scan_into_wizard(ctx, p));
    });
}

/// Разобрать источник в worker'е и заселить мастер результатом.
///
/// Скан дешёвый (метаданные файлов + `config.json` + safetensors-заголовки),
/// но на сетевой ФС или холодном кеше он может занять заметное время —
/// поэтому не на UI-потоке.
pub fn scan_into_wizard(ctx: SynExplorerCtx, path: PathBuf) {
    let wizard = ctx.wizard;
    wizard.scanning.set(true);
    let models_dir = ctx.models_dir.get_untracked();
    std::thread::spawn(move || {
        match synaptix_bundle::pack_plan::PackPlan::scan(&path) {
            Ok(plan) => {
                let layers = main_component_layers(&plan);
                let out = default_out_path(&plan, &models_dir);
                let plan = Arc::new(plan);
                run_on_main_thread(move || {
                    wizard.adopt(plan, layers);
                    wizard.out_path.update(|v| *v = Some(out));
                });
            }
            Err(e) => {
                let msg = e.to_string();
                run_on_main_thread(move || {
                    wizard.scanning.set(false);
                    ctx.show_error(tr!("explorer.error.scan_failed.title"), msg);
                });
            }
        }
    });
}

/// Куда положить бандл по умолчанию: в каталог моделей приложения, если он
/// существует, иначе рядом с источником. Первое важнее — именно там его
/// ищут ноды и агент, и собранный пакет сразу оказывается «на месте».
fn default_out_path(plan: &synaptix_bundle::pack_plan::PackPlan, models_dir: &Path) -> PathBuf {
    if models_dir.is_dir() {
        return plan.suggested_out(models_dir);
    }
    let near = if plan.root.is_dir() {
        plan.root.parent().unwrap_or(&plan.root).to_path_buf()
    } else {
        plan.root
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    };
    plan.suggested_out(&near)
}

/// Состав главного компонента — инспектор слоёв внутри мастера.
///
/// Заголовки читаются у **всех** шардов компонента, а не только у первого:
/// в первом шарде Qwen3.8 лежит vision-башня и часть слоёв, и по нему одному
/// «состав модели» выходил бы «vision 86 %» вместо честных полутора процентов.
/// Цена — по одному короткому чтению на шард (веса не трогаются).
fn main_component_layers(
    plan: &synaptix_bundle::pack_plan::PackPlan,
) -> Option<ComponentLayers> {
    use synaptix_bundle::inspect;
    let comp = plan.components.iter().find(|c| c.enabled)?;
    let mut tensors: Vec<inspect::TensorInfo> = Vec::new();
    for shard in &comp.paths {
        match inspect::read_header_file(shard) {
            Ok(mut t) => tensors.append(&mut t),
            // Битый шард не повод остаться совсем без состава: показываем
            // то, что прочиталось, — это подсказка, а не контрольная сумма.
            Err(e) => tracing::warn!(target: "syn-explorer", shard = %shard.display(), error = %e, "не прочитан заголовок шарда"),
        }
    }
    if tensors.is_empty() {
        return None;
    }
    let hint = format!("{} {} {}", comp.name, plan.meta.purpose, plan.meta.id);
    let mut by_role: Vec<_> = inspect::bytes_by_role(&tensors, Some(&hint))
        .into_iter()
        .collect();
    by_role.sort_by(|a, b| b.1.dense.cmp(&a.1.dense));
    Some(ComponentLayers {
        component: comp.name.clone(),
        tensor_count: tensors.len(),
        bytes: tensors.iter().map(|t| t.bytes).sum(),
        groups: inspect::group_tensors(&tensors, Some(&hint)),
        by_role,
    })
}

/// Запустить упаковку из мастера.
pub fn start_packing(ctx: SynExplorerCtx) {
    if let Some(err) = ctx.wizard.validation_error() {
        ctx.show_error(tr!("explorer.error.missing_data.title"), err);
        return;
    }
    let Some(plan) = ctx.wizard.effective_plan() else {
        return;
    };
    let Some(out) = ctx.wizard.out_path.get_untracked() else {
        return;
    };
    let quant = ctx.wizard.quant_decision();
    // Квантование теряет точность безвозвратно. Разрешить вместе с ним
    // удаление исходников значило бы дать одним кликом уничтожить
    // единственную полную копию весов.
    if !quant.is_empty() && ctx.wizard.delete_sources.get_untracked() {
        ctx.show_error(
            tr!("explorer.error.quant_delete.title"),
            tr!("explorer.error.quant_delete.message"),
        );
        return;
    }
    if !quant.is_empty() && !super::quant_pack::cuda_available() {
        ctx.show_error(
            tr!("explorer.error.quant_no_cuda.title"),
            tr!("explorer.error.quant_no_cuda.message"),
        );
        return;
    }
    // Пакет весит столько, сколько обещано на шаге со слоями, — по этой же
    // цифре проверяется место на разделе, иначе упаковку 175-гигабайтного
    // пакета отказывались начинать из-за 335 ГБ исходников.
    let payload_estimate = (!quant.is_empty())
        .then(|| ctx.wizard.estimated_payload().map(|(_, quantized)| quantized))
        .flatten();
    let opts = bundle_io::PackOptions {
        quant,
        payload_estimate,
        delete_sources: ctx.wizard.delete_sources.get_untracked(),
        sha256: ctx.wizard.sha256.get_untracked(),
        blake3: ctx.wizard.blake3.get_untracked(),
        cdir_json: ctx.wizard.cdir_json.get_untracked(),
    };
    bundle_io::create_from_plan_async(ctx, plan, out, opts);
}

/// Выбрать, куда писать `.syn`.
pub fn pick_out_path(wizard: PackWizard) {
    let suggested = wizard.id.get_untracked();
    let suggested = if suggested.is_empty() {
        "package.syn".to_string()
    } else {
        format!("{suggested}.syn")
    };
    let start_dir = wizard
        .out_path
        .get_untracked()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    std::thread::spawn(move || {
        let mut dlg = rfd::FileDialog::new()
            .add_filter("Syn bundle", &["syn"])
            .set_title(tr!("explorer.dialog.rfd.pick_out_title"))
            .set_file_name(&suggested);
        if let Some(d) = start_dir {
            dlg = dlg.set_directory(d);
        }
        if let Some(p) = dlg.save_file() {
            run_on_main_thread(move || {
                wizard.out_path.update(|v| *v = Some(p));
            });
        }
    });
}

/// Включить/выключить компонент или файл в составе будущего бандла.
pub fn toggle_component(wizard: PackWizard, index: usize, on: bool) {
    wizard.components_enabled.update(|v| {
        if let Some(slot) = v.get_mut(index) {
            *slot = on;
        }
    });
}

pub fn toggle_aux(wizard: PackWizard, index: usize, on: bool) {
    wizard.aux_enabled.update(|v| {
        if let Some(slot) = v.get_mut(index) {
            *slot = on;
        }
    });
}

/// Переименовать компонент. Имя — суффикс чанка `tensors:<name>`, по нему
/// компонент ищут загрузчики, поэтому правка доступна только эксперту.
pub fn set_component_name(wizard: PackWizard, index: usize, name: String) {
    wizard.plan.update(|slot| {
        if let Some(plan) = slot.as_mut() {
            let mut edited = (**plan).clone();
            if let Some(c) = edited.components.get_mut(index) {
                c.name = name.trim().to_string();
            }
            *plan = Arc::new(edited);
        }
    });
}

/// Префикс имён тензоров компонента. Пусто — без пространства имён.
pub fn set_component_prefix(wizard: PackWizard, index: usize, prefix: String) {
    wizard.plan.update(|slot| {
        if let Some(plan) = slot.as_mut() {
            let mut edited = (**plan).clone();
            if let Some(c) = edited.components.get_mut(index) {
                c.prefix = prefix.trim().to_string();
            }
            *plan = Arc::new(edited);
        }
    });
}

/// Назначение файла внутри бандла. Загрузчики читают только `inference`.
pub fn set_aux_tag(wizard: PackWizard, index: usize, tag: &str) {
    let tag = match tag {
        "doc" => FileTag::Doc,
        "example" => FileTag::Example,
        "asset" => FileTag::Asset,
        _ => FileTag::Inference,
    };
    wizard.plan.update(|slot| {
        if let Some(plan) = slot.as_mut() {
            let mut edited = (**plan).clone();
            if let Some(f) = edited.aux.get_mut(index) {
                f.tag = tag;
            }
            *plan = Arc::new(edited);
        }
    });
}

/// Шаг мастера. Границы задаются вызывающим — шагов ровно три.
pub fn goto_step(wizard: PackWizard, step: usize) {
    wizard.step.set(step.min(2));
}

/// Активный таб → Preview. Используется при клике в TreeView, чтобы сразу
/// показать содержимое выбранного файла.
pub fn switch_to_preview(ctx: SynExplorerCtx) {
    if ctx.current_tab.get_untracked() != TabKind::Preview {
        ctx.current_tab.set(TabKind::Preview);
    }
}
