//! Управление закладками-папками для SynExplorer и сканирование `.syn`
//! файлов в выбранной папке.

use std::path::{Path, PathBuf};

use syngui::async_runtime::run_on_main_thread;

use super::state::{SynExplorerCtx, SynFileEntry};

/// Добавить путь в закладки. Идемпотентно — повторное добавление того же пути
/// — no-op. Не удаляет existing entries. Обновление signal'ов делается через
/// `run_on_main_thread`: функция может вызываться и с UI, и с rfd-worker'а.
pub fn add_bookmark(ctx: SynExplorerCtx, path: PathBuf) {
    if !path.is_dir() {
        let p = path.display().to_string();
        run_on_main_thread(move || {
            ctx.show_error(
                "Закладка не добавлена",
                format!("«{}» не является папкой.", p),
            );
        });
        return;
    }
    let path_for_select = path.clone();
    run_on_main_thread(move || {
        let already = ctx
            .bookmarks
            .get_untracked()
            .iter()
            .any(|p| p == &path);
        if already {
            return;
        }
        ctx.bookmarks.update(|v| v.push(path.clone()));
        select_folder(ctx, path_for_select);
    });
}

/// Убрать закладку по индексу. Если она была выбранной — сбрасываем
/// selected_folder и folder_entries.
pub fn remove_bookmark(ctx: SynExplorerCtx, idx: usize) {
    let bookmarks = ctx.bookmarks.get_untracked();
    let Some(removed) = bookmarks.get(idx).cloned() else {
        return;
    };
    ctx.bookmarks.update(|v| {
        v.remove(idx);
    });
    if ctx.selected_folder.get_untracked().as_ref() == Some(&removed) {
        ctx.selected_folder.set(None);
        ctx.folder_entries.set(Vec::new());
    }
}

/// Выбрать закладку (или произвольную папку) — обновляет `selected_folder` и
/// `folder_entries`. На ошибки чтения — Error-диалог.
pub fn select_folder(ctx: SynExplorerCtx, path: PathBuf) {
    if !path.is_dir() {
        ctx.show_error(
            "Папка недоступна",
            format!("«{}» не существует или не папка.", path.display()),
        );
        ctx.selected_folder.set(None);
        ctx.folder_entries.set(Vec::new());
        return;
    }
    match scan_folder_for_syn(&path) {
        Ok(entries) => {
            ctx.selected_folder.set(Some(path));
            ctx.folder_entries.set(entries);
        }
        Err(e) => {
            ctx.show_error("Не удалось прочитать папку", e);
            ctx.selected_folder.set(Some(path));
            ctx.folder_entries.set(Vec::new());
        }
    }
}

/// Перечитать `folder_entries` для текущей выбранной папки. No-op если
/// ничего не выбрано.
pub fn refresh_selected(ctx: SynExplorerCtx) {
    let Some(folder) = ctx.selected_folder.get_untracked() else {
        return;
    };
    select_folder(ctx, folder);
}

/// Сканировать папку на наличие `.syn`-файлов. Не открывает их (тяжёлый mmap
/// — только при клике). Сортировка по имени для стабильного UI-порядка.
pub fn scan_folder_for_syn(dir: &Path) -> Result<Vec<SynFileEntry>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut out: Vec<SynFileEntry> = Vec::new();
    for ent in rd.flatten() {
        let p = ent.path();
        if !p.is_file() {
            continue;
        }
        let ext_is_syn = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.eq_ignore_ascii_case("syn"))
            .unwrap_or(false);
        if !ext_is_syn {
            continue;
        }
        let display_name = p
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| p.display().to_string());
        let size = std::fs::metadata(&p).ok().map(|m| m.len());
        out.push(SynFileEntry {
            path: p,
            display_name,
            size,
        });
    }
    out.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Ok(out)
}
