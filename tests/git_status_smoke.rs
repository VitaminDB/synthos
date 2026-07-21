//! Smoke-тест для `git_status::compute` на реальном workspace-репо.
//! Проверяет, что:
//! - `compute` не паникует на текущем syngui-репо;
//! - возвращает непустую workdir;
//! - HashMap files+folders валидны (никаких bogus-путей).
//!
//! Не проверяем конкретное содержимое — оно меняется на каждом коммите.

use synthos::pages::code_editor::git_status;
use std::path::PathBuf;

#[test]
fn compute_does_not_panic_on_workspace_repo() {
    // Запускаем тест из workspace root: synthos живёт в app/synthos/,
    // workspace root — на 2 уровня выше.
    let here = std::env::current_dir().expect("cwd");
    // CARGO_MANIFEST_DIR для теста = app/synthos/. Поднимаемся на 2.
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .expect("workspace root");

    let map = git_status::compute(&workspace_root);

    // syngui — git-репо, поэтому workdir должен быть Some.
    assert!(map.workdir.is_some(), "syngui — git-репо, ожидаем Some workdir");

    // Все ключи files и folders должны быть абсолютными путями внутри
    // workdir.
    if let Some(wd) = &map.workdir {
        for path in map.files.keys() {
            assert!(
                path.starts_with(wd),
                "file path {path:?} вне workdir {wd:?}"
            );
        }
        for path in map.folders.keys() {
            assert!(
                path.starts_with(wd),
                "folder path {path:?} вне workdir {wd:?}"
            );
            // Папки workdir не подсвечиваем — workdir исключён из rollup.
            assert_ne!(path, wd, "сам workdir не должен быть в folders");
        }
    }

    // На случай отладки.
    let _ = here;
}
