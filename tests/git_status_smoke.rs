//! Smoke-тест для `git_status::compute` на реальном репозитории.
//! Проверяет, что:
//! - `compute` не паникует на репозитории synthos;
//! - возвращает непустую workdir;
//! - HashMap files+folders валидны (никаких bogus-путей).
//!
//! Не проверяем конкретное содержимое — оно меняется на каждом коммите.

use synthos::pages::code_editor::git_status;
use std::path::PathBuf;

#[test]
fn compute_does_not_panic_on_workspace_repo() {
    // Берём корень самого crate'а: он и есть git-репо synthos. Раньше тест
    // поднимался на два уровня вверх — это была раскладка `app/synthos/`
    // внутри общего workspace'а. После переезда проекта два уровня вверх
    // ведут в каталог без `.git`, и тест падал на первом же assert'е.
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let map = git_status::compute(&repo_root);

    assert!(map.workdir.is_some(), "synthos — git-репо, ожидаем Some workdir");

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
}
