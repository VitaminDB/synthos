//! Файловые операции, инициируемые из контекстного меню.
//!
//! Все ошибки синхронные, через `std::fs`, без unwrap'ов — любая ошибка
//! ввода-вывода логируется через `eprintln!` и выводится в snackbar
//! ([`CodeEditorCtx::show_notice`]). Состояние сессии (дерево, открытые
//! файлы, активный файл) обновляется атомарно после успеха операции.
//!
//! Все функции принимают `CodeSession` (Copy) первым аргументом — так же
//! как [`super::state`]: общий стиль, удобно вызывать из callback'ов.

use std::path::{Path, PathBuf};

use syngui::context_provider::use_context;
use syngui::tr;

use super::fs_ops;
use super::state::{self, CodeEditorCtx, CodeSession};

/// Создать пустой файл `name` внутри `parent`. Если файл уже существует —
/// открыть его. Дерево обновляется только для родителя (через
/// [`refresh_dir_in_tree`]) — не нужно пересобирать всё дерево.
pub fn create_file(session: CodeSession, parent: PathBuf, name: String) {
    let name = name.trim();
    if name.is_empty() {
        notice(tr!("code.fs.error.empty_file_name"));
        return;
    }
    if name.contains('/') || name.contains('\\') {
        notice(tr!("code.fs.error.invalid_name_file"));
        return;
    }
    let target = parent.join(name);
    if target.exists() {
        eprintln!("[code-editor] create_file: уже существует {:?}", target);
        // Не ошибка — просто откроем.
        state::open_file(session, target);
        return;
    }
    if let Err(e) = std::fs::write(&target, "") {
        eprintln!("[code-editor] create_file {:?}: {e}", target);
        notice(tr!("code.fs.error.create_file_failed", error = e));
        return;
    }
    refresh_dir_in_tree(session, &parent);
    state::open_file(session, target);
}

/// Создать пустой каталог `name` внутри `parent`.
pub fn create_folder(session: CodeSession, parent: PathBuf, name: String) {
    let name = name.trim();
    if name.is_empty() {
        notice(tr!("code.fs.error.empty_folder_name"));
        return;
    }
    if name.contains('/') || name.contains('\\') {
        notice(tr!("code.fs.error.invalid_name"));
        return;
    }
    let target = parent.join(name);
    if target.exists() {
        notice(tr!("code.fs.error.already_exists", name = name));
        return;
    }
    if let Err(e) = std::fs::create_dir(&target) {
        eprintln!("[code-editor] create_folder {:?}: {e}", target);
        notice(tr!("code.fs.error.create_folder_failed", error = e));
        return;
    }
    refresh_dir_in_tree(session, &parent);
}

/// Переименовать файл или папку. `new_name` — basename без слэшей.
/// Обновляет `open_files` (если переименовываемый файл был открыт) и
/// `active_file`. Папки переименовываются вместе со всем содержимым —
/// все открытые файлы под ней получают новый префикс пути.
pub fn rename(session: CodeSession, old: PathBuf, new_name: String) {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        notice(tr!("code.fs.error.empty_name"));
        return;
    }
    if new_name.contains('/') || new_name.contains('\\') {
        notice(tr!("code.fs.error.invalid_name"));
        return;
    }
    let Some(parent) = old.parent().map(Path::to_path_buf) else {
        notice(tr!("code.fs.error.rename_root"));
        return;
    };
    let new_path = parent.join(new_name);
    if new_path == old {
        return;
    }
    if new_path.exists() {
        notice(tr!("code.fs.error.already_exists", name = new_name));
        return;
    }
    if let Err(e) = std::fs::rename(&old, &new_path) {
        eprintln!("[code-editor] rename {:?} -> {:?}: {e}", old, new_path);
        notice(tr!("code.fs.error.rename_failed", error = e));
        return;
    }

    // Обновить open_files / active_file / file_contents / disk_contents:
    // переписать ключи под новый prefix (если переименовали папку, или сам файл).
    rebase_open_files(session, &old, &new_path);
    refresh_dir_in_tree(session, &parent);
}

/// Удалить файл или папку. `confirmed=true` — пользователь подтвердил
/// удаление в диалоге. Без подтверждения функция не вызывается (UI шлёт
/// `confirmed=true` после клика «Удалить»).
pub fn delete(session: CodeSession, path: PathBuf) {
    let res = if path.is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    };
    if let Err(e) = res {
        eprintln!("[code-editor] delete {:?}: {e}", path);
        notice(tr!("code.fs.error.delete_failed", error = e));
        return;
    }

    // Закрыть все открытые файлы под удалённой папкой / удалённый файл сам.
    let to_close: Vec<PathBuf> = session
        .open_files
        .get_untracked()
        .into_iter()
        .filter(|p| p == &path || p.starts_with(&path))
        .collect();
    for p in to_close {
        state::close_file(session, p);
    }

    let parent = path.parent().map(Path::to_path_buf);
    if let Some(p) = parent {
        refresh_dir_in_tree(session, &p);
    }
}

/// Скопировать абсолютный путь в системный буфер обмена. Использует
/// `syngui::clipboard::copy` — обёртку над `arboard` с graceful fallback
/// на платформах без clipboard-поддержки (warn-лог). Ошибки внутри
/// уже логируются через `log::warn!`.
pub fn copy_path_to_clipboard(path: &Path) {
    let s = path.display().to_string();
    syngui::clipboard::copy(&s);
    notice(tr!("code.fs.notice.copied", value = s));
}

/// Открыть путь в системном файловом менеджере (через `xdg-open` /
/// `open` / `explorer`). Если путь — файл, открывается его родитель;
/// файл-менеджер сам выделяет файл (поведение `xdg-open` зависит от DE,
/// для надёжности всегда открываем родительскую папку).
pub fn reveal_in_files(path: &Path) {
    let target = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    match crate::paths::open_with_system(target) {
        Ok(_) => {}
        Err(e) => {
            eprintln!("[code-editor] reveal_in_files {:?}: {e}", target);
            notice(tr!("code.fs.error.open_failed", error = e));
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn notice(msg: impl Into<String>) {
    let ctx = use_context::<CodeEditorCtx>();
    ctx.show_notice(msg);
}

/// Перечитать содержимое каталога `dir` в `session.tree_nodes`. Вызывается
/// после fs-операций (create / rename / delete), чтобы дерево отразило
/// изменения без перезагрузки всего проекта. Если `dir` ещё не был
/// раскрыт (отсутствует в `loaded_dirs`) — просто помечаем для
/// переподгрузки на следующем toggle.
fn refresh_dir_in_tree(session: CodeSession, dir: &Path) {
    // Если каталог не был загружен — нечего обновлять (его дети пока
    // placeholder), при следующем раскрытии прочтётся актуальное содержимое.
    if !session.loaded_dirs.get_untracked().contains(dir) {
        return;
    }
    let id = fs_ops::id_for(dir);
    // Fail-closed чтение: при транзиентной ошибке не трогаем дерево, иначе
    // неполный снимок стёр бы живые узлы (см. fs_ops::read_level).
    let new_kids = match fs_ops::read_dir_to_nodes_checked(dir) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("[code-editor] refresh_dir_in_tree {:?}: {e}", dir);
            return;
        }
    };
    // root читаем ДО update — внутри update RUNTIME уже borrow_mut,
    // и любой get_untracked внутри замыкания паникует с
    // "RefCell already mutably borrowed" (signal.rs:196).
    let is_root = session
        .root_folder
        .get_untracked()
        .map(|r| dir == r.as_path())
        .unwrap_or(false);
    session.tree_nodes.update(|nodes| {
        if is_root {
            *nodes = fs_ops::merge_preserve_expanded(new_kids, nodes);
            return;
        }
        let mut taken = Some(new_kids);
        let _ = fs_ops::replace_children_and_expand(nodes, &id, &mut taken);
    });
}

/// При переименовании пути `old → new_path` обновить все ключи `open_files`,
/// `file_contents`, `disk_contents` и `active_file`, начинающиеся с `old`.
/// Это важно для папок: переименование `src/` → `source/` обновляет все
/// открытые файлы внутри (`src/main.rs` → `source/main.rs`).
fn rebase_open_files(session: CodeSession, old: &Path, new_path: &Path) {
    let rebase = |p: &PathBuf| -> Option<PathBuf> {
        if p == old {
            Some(new_path.to_path_buf())
        } else if let Ok(rest) = p.strip_prefix(old) {
            Some(new_path.join(rest))
        } else {
            None
        }
    };

    session.open_files.update(|v| {
        for p in v.iter_mut() {
            if let Some(np) = rebase(p) {
                *p = np;
            }
        }
    });
    session.active_file.update(|opt| {
        if let Some(p) = opt {
            if let Some(np) = rebase(p) {
                *opt = Some(np);
            }
        }
    });
    session.file_contents.update(|m| {
        let keys: Vec<PathBuf> = m.keys().cloned().collect();
        for k in keys {
            if let Some(nk) = rebase(&k) {
                if let Some(v) = m.remove(&k) {
                    m.insert(nk, v);
                }
            }
        }
    });
    session.disk_contents.update(|m| {
        let keys: Vec<PathBuf> = m.keys().cloned().collect();
        for k in keys {
            if let Some(nk) = rebase(&k) {
                if let Some(v) = m.remove(&k) {
                    m.insert(nk, v);
                }
            }
        }
    });
    // Перерисовать редактор, если активный файл переименовался.
    session.editor_gen.update(|n| *n = n.wrapping_add(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebase_helper_rewrites_subpaths() {
        let old = PathBuf::from("/p/src");
        let new_path = PathBuf::from("/p/source");
        let rebase = |p: &PathBuf| -> Option<PathBuf> {
            if p == &old {
                Some(new_path.clone())
            } else if let Ok(rest) = p.strip_prefix(&old) {
                Some(new_path.join(rest))
            } else {
                None
            }
        };
        assert_eq!(rebase(&PathBuf::from("/p/src")).unwrap(), PathBuf::from("/p/source"));
        assert_eq!(
            rebase(&PathBuf::from("/p/src/lib.rs")).unwrap(),
            PathBuf::from("/p/source/lib.rs")
        );
        assert_eq!(rebase(&PathBuf::from("/p/other")), None);
    }
}
