//! Пути в человекочитаемом виде.
//!
//! Сокращение по ширине (`~/…/2027/synthos`) делает [`syngui::Elide::Middle`]
//! в момент отрисовки — только там известна реальная ширина панели. Здесь
//! живёт та часть, что от ширины не зависит.

use std::path::Path;

/// Домашний каталог пользователя из окружения.
fn home() -> Option<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .filter(|h| !h.is_empty())
}

/// Открыть файл или каталог системным приложением (`xdg-open`, `open`,
/// `explorer`). Процесс дожидается фоновый поток: без `wait` каждый вызов
/// оставлял зомби до выхода приложения.
pub fn open_with_system(target: &Path) -> std::io::Result<()> {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(cmd).arg(target).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Путь, введённый руками: `~` и `~/…` раскрываются в домашний каталог
/// (подсказки полей показывают `~/Downloads/…`, а `File::create` понимает
/// `~` буквально — получался каталог `./~`). Пробелы по краям срезаются.
pub fn expand_home(raw: &str) -> std::path::PathBuf {
    let raw = raw.trim();
    match (home(), raw) {
        (Some(h), "~") => std::path::PathBuf::from(h),
        (Some(h), r) if r.starts_with("~/") => std::path::PathBuf::from(h).join(&r[2..]),
        _ => std::path::PathBuf::from(raw),
    }
}

/// Заменяет домашний каталог на `~`: `/home/master/Projects` → `~/Projects`.
/// Путь вне `$HOME` возвращается как есть.
pub fn pretty(path: &Path) -> String {
    pretty_str(&path.display().to_string())
}

/// То же для готовой строки — когда путь уже склеен с подписью.
pub fn pretty_str(text: &str) -> String {
    let Some(home) = home() else {
        return text.to_string();
    };
    let home = home.trim_end_matches('/');
    if text == home {
        return "~".to_string();
    }
    match text.strip_prefix(home).filter(|rest| rest.starts_with('/')) {
        Some(rest) => format!("~{rest}"),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_home_resolves_tilde() {
        let Some(h) = home() else { return };
        assert_eq!(expand_home("~/Downloads/a.wav"), Path::new(&h).join("Downloads/a.wav"));
        assert_eq!(expand_home(" ~ "), Path::new(&h));
        assert_eq!(expand_home("/tmp/x"), Path::new("/tmp/x"));
        assert_eq!(expand_home("~user/x"), Path::new("~user/x"));
    }

    fn with_home<T>(value: &str, f: impl FnOnce() -> T) -> T {
        let prev = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", value) };
        let out = f();
        match prev {
            Some(p) => unsafe { std::env::set_var("HOME", p) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        out
    }

    #[test]
    fn replaces_home_with_tilde() {
        with_home("/home/master", || {
            assert_eq!(pretty(Path::new("/home/master/Projects/2027")), "~/Projects/2027");
        });
    }

    #[test]
    fn home_itself_becomes_tilde() {
        with_home("/home/master", || {
            assert_eq!(pretty(Path::new("/home/master")), "~");
        });
    }

    #[test]
    fn leaves_paths_outside_home_alone() {
        with_home("/home/master", || {
            assert_eq!(pretty(Path::new("/etc/fstab")), "/etc/fstab");
        });
    }

    /// `/home/master2` не внутри `/home/master` — префикс совпадает, но
    /// границы сегмента нет.
    #[test]
    fn does_not_match_a_sibling_with_the_same_prefix() {
        with_home("/home/master", || {
            assert_eq!(pretty(Path::new("/home/master2/x")), "/home/master2/x");
        });
    }

    #[test]
    fn tolerates_trailing_slash_in_home() {
        with_home("/home/master/", || {
            assert_eq!(pretty(Path::new("/home/master/Projects")), "~/Projects");
        });
    }
}
