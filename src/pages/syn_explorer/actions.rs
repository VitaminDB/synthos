//! Высокоуровневые действия страницы — обёртки над `bundle_io` + `bookmarks`
//! с интеграцией rfd (native file dialogs). Все функции не падают: каждая
//! ошибка превращается в Error-диалог или notification.
//!
//! rfd работает асинхронно (futures::Future) — на синхронном UI-callback'е мы
//! проще запускаем blocking-вариант в отдельном потоке. Это не блокирует
//! event-loop, потому что rfd на Linux использует xdg-portal (D-Bus,
//! ожидание пользователя — естественно блокирующее в worker'е).

use std::path::PathBuf;
use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use synaptix_bundle::FileTag;

use super::bookmarks;
use super::bundle_io;
use super::state::{
    DialogKind, NewPackageComponent, NewPackageForm, OpenBundle, PendingOp, SynExplorerCtx, TabKind,
};

/// Открыть .syn через native file dialog. Воркер запускает rfd blocking, по
/// результату вызывает `bundle_io::open_async` (на main-thread).
pub fn pick_and_open_bundle(ctx: SynExplorerCtx) {
    std::thread::spawn(move || {
        let path = rfd::FileDialog::new()
            .add_filter("Syn bundle", &["syn"])
            .set_title("Открыть .syn пакет")
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
            .set_title("Добавить папку с .syn в закладки")
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
        ctx.show_error("Не выбран пакет", "Сначала откройте `.syn` пакет.");
        return;
    };
    std::thread::spawn(move || {
        let picked = rfd::FileDialog::new()
            .set_title("Импортировать файл в пакет")
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
                ctx.show_error("Импорт файла", "Не удалось определить имя файла.");
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
                    ctx.show_error("Импорт файла", msg);
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
            "Извлечь файл",
            "Сначала выберите файл в TreeView пакета справа.",
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
            .set_title(format!("Сохранить «{leaf}» как…"))
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
                        ctx.show_error("Извлечь файл", msg);
                    });
                }
            }
            None => {
                run_on_main_thread(move || {
                    ctx.show_error(
                        "Извлечь файл",
                        format!("Не удалось прочитать `{name}` из пакета."),
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
        ctx.show_error("Удалить файл", "Сначала выберите файл в правой панели.");
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

/// Открыть диалог создания нового пакета. Сначала сбрасываем форму
/// (общая для всего lifetime ctx).
pub fn request_create_bundle(ctx: SynExplorerCtx) {
    ctx.reset_new_package_form();
    ctx.open_dialog(DialogKind::NewPackage);
}

/// Запустить создание из формы NewPackage (worker через bundle_io::create_async).
pub fn create_bundle(ctx: SynExplorerCtx) {
    bundle_io::create_async(ctx, ctx.new_package_form);
}

/// Выбрать папку-источник для конкретного компонента NewPackage.
///
/// `is_first` — компонент с индексом 0. Только для него мы заполняем
/// автоматически out_path и id (компоненты 2..n — это дополнительные
/// tensor-источники, у них своё имя но общие meta).
pub fn pick_source_dir_for_component(
    form: NewPackageForm,
    component: NewPackageComponent,
    is_first: bool,
) {
    std::thread::spawn(move || {
        let p = match rfd::FileDialog::new()
            .set_title("Папка с safetensors-моделью")
            .pick_folder()
        {
            Some(p) => p,
            None => return,
        };
        run_on_main_thread(move || {
            if is_first {
                if form.out_path.get_untracked().is_none() {
                    if let Some(parent) = p.parent() {
                        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                            form.out_path
                                .set(Some(parent.join(format!("{name}.syn"))));
                        }
                    }
                }
                if form.id.get_untracked().trim().is_empty() {
                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                        form.id.set(name.to_string());
                    }
                }
            }
            component.source_dir.set(Some(p));
        });
    });
}

/// Добавить пустой компонент (multi-tensor бандл). По умолчанию имя
/// `comp{N}` — пользователь редактирует в TextField.
pub fn add_component(form: NewPackageForm) {
    form.components.update(|v| {
        let next_idx = v.len();
        v.push(NewPackageComponent::new(&format!("comp{next_idx}")));
    });
}

/// Удалить компонент по индексу. Не даём удалить последний — пакет должен
/// иметь хотя бы один tensors-чанк (UI блокирует кнопку delete).
pub fn remove_component(form: NewPackageForm, index: usize) {
    form.components.update(|v| {
        if v.len() > 1 && index < v.len() {
            v.remove(index);
        }
    });
}

/// Выбрать out-path для NewPackage (save_file).
pub fn pick_out_path_for_new(form: NewPackageForm) {
    let suggested = form.id.get_untracked();
    let suggested = if suggested.is_empty() {
        "package.syn".to_string()
    } else {
        format!("{suggested}.syn")
    };
    std::thread::spawn(move || {
        let path = rfd::FileDialog::new()
            .add_filter("Syn bundle", &["syn"])
            .set_title("Куда сохранить .syn")
            .set_file_name(&suggested)
            .save_file();
        if let Some(p) = path {
            run_on_main_thread(move || {
                form.out_path.set(Some(p));
            });
        }
    });
}

/// Активный таб → Preview. Используется при клике в TreeView, чтобы сразу
/// показать содержимое выбранного файла.
pub fn switch_to_preview(ctx: SynExplorerCtx) {
    if ctx.current_tab.get_untracked() != TabKind::Preview {
        ctx.current_tab.set(TabKind::Preview);
    }
}
