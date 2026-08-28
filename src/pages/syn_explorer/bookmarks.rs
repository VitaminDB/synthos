//! Управление закладками-папками для SynExplorer и сканирование `.syn`
//! файлов в выбранной папке.

use std::path::{Path, PathBuf};

use syngui::async_runtime::run_on_main_thread;
use syngui::tr;

use super::state::{SourceEntry, SynExplorerCtx, SynFileEntry};

/// Добавить путь в закладки. Идемпотентно — повторное добавление того же пути
/// — no-op. Не удаляет existing entries. Обновление signal'ов делается через
/// `run_on_main_thread`: функция может вызываться и с UI, и с rfd-worker'а.
pub fn add_bookmark(ctx: SynExplorerCtx, path: PathBuf) {
    if !path.is_dir() {
        let p = path.display().to_string();
        run_on_main_thread(move || {
            ctx.show_error(
                tr!("explorer.error.bookmark_not_added.title"),
                tr!("explorer.error.bookmark_not_added.message", path = p),
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
        ctx.folder_sources.set(Vec::new());
    }
}

/// Выбрать закладку (или произвольную папку) — обновляет `selected_folder` и
/// `folder_entries`. На ошибки чтения — Error-диалог.
pub fn select_folder(ctx: SynExplorerCtx, path: PathBuf) {
    if !path.is_dir() {
        ctx.show_error(
            tr!("explorer.error.folder_unavailable.title"),
            tr!("explorer.error.folder_unavailable.message", path = path.display()),
        );
        ctx.selected_folder.set(None);
        ctx.folder_entries.set(Vec::new());
        ctx.folder_sources.set(Vec::new());
        return;
    }
    match scan_folder(&path) {
        Ok((bundles, sources)) => {
            ctx.selected_folder.set(Some(path));
            ctx.folder_entries.set(bundles);
            ctx.folder_sources.set(sources);
        }
        Err(e) => {
            ctx.show_error(tr!("explorer.error.read_folder.title"), e);
            ctx.selected_folder.set(Some(path));
            ctx.folder_entries.set(Vec::new());
            ctx.folder_sources.set(Vec::new());
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

/// Сканировать папку: готовые `.syn` и модели, которые ещё можно упаковать.
///
/// Раньше здесь искались только `.syn`, и папка с моделью выглядела пустой —
/// «нет `.syn` файлов» вместо очевидного «вот модель, вот кнопка упаковать».
/// Разбор делает `pack_plan::scan_collection`: он не открывает веса, только
/// перечисляет файлы и читает `config.json`, поэтому остаётся мгновенным
/// даже на каталоге с сотнями гигабайт.
pub fn scan_folder(dir: &Path) -> Result<(Vec<SynFileEntry>, Vec<SourceEntry>), String> {
    let items = synaptix_bundle::pack_plan::scan_collection(dir).map_err(|e| e.to_string())?;
    let mut bundles: Vec<SynFileEntry> = Vec::new();
    let mut sources: Vec<SourceEntry> = Vec::new();
    for item in items {
        match item {
            synaptix_bundle::pack_plan::FoundItem::Bundle { path, name, bytes } => {
                bundles.push(SynFileEntry {
                    path,
                    display_name: name,
                    size: Some(bytes),
                });
            }
            synaptix_bundle::pack_plan::FoundItem::Source(c) => {
                sources.push(SourceEntry {
                    path: c.path,
                    display_name: c.name,
                    bytes: c.bytes,
                    shard_count: c.shard_count,
                    component_count: c.component_count,
                    arch: c.arch,
                });
            }
        }
    }
    bundles.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    sources.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Ok((bundles, sources))
}
