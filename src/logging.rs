//! Инициализация tracing: stderr (для разработчика) + rolling file
//! appender (для post-mortem отладки на машине пользователя).
//!
//! Пути логов:
//! - Linux:   `$XDG_STATE_HOME/synthos/logs/` (по умолчанию `~/.local/state/synthos/logs/`).
//! - macOS:   `~/.local/state/synthos/logs/` (тот же путь, не лезем в `~/Library/Logs/`).
//! - Windows: `%LOCALAPPDATA%\synthos\logs` или `<HOME>\.local\state\synthos\logs`.
//!
//! Файлы ротируются по дням: `synthos.log.YYYY-MM-DD`. `tracing-appender`
//! сам открывает новый файл каждый день и оставляет старые рядом — без
//! автоматической очистки. На длительной перспективе пользователь может
//! запустить `find ~/.local/state/synthos/logs -mtime +30 -delete` или мы
//! добавим чистильщика, если это станет проблемой.
//!
//! Уровни:
//! - **stderr**: `INFO` (можно переопределить `RUST_LOG=...`);
//! - **файл**:  `DEBUG` (всегда — это и есть смысл файла: на ошибке мы
//!   хотим видеть все debug-события без перезапуска).
//!
//! Контракт: [`init`] вызывается один раз из `run_desktop`/`android_main`
//! ДО любого `tracing!`-вызова. Возвращает [`LogGuard`], который **обязан**
//! жить до конца процесса — non-blocking writer'у нужен живой guard,
//! иначе он дропается и хвост логов теряется. Пихаем guard в `OnceLock`,
//! чтобы он жил столько же, сколько и сам процесс.

use std::path::PathBuf;
use std::sync::OnceLock;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Глобально живущий guard non-blocking-writer'а. См. doc-коммент модуля.
static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

/// Инициализирует tracing-subscriber.
///
/// Идемпотентно: повторный вызов вернёт ошибку `try_init` (молча
/// проигнорируется), guard в `OnceLock` уже стоит — повторно записать не
/// получится.
///
/// `tracing!`-вызовы до этой функции теряются (subscriber по умолчанию
/// игнорирует события без подписчика).
pub fn init() {
    let log_dir = log_dir_path();
    let mut file_layer_failed: Option<String> = None;

    // Файловый layer: rolling daily, без ANSI, debug-уровень.
    let file_layer = match prepare_file_writer(&log_dir) {
        Ok((non_blocking, guard)) => {
            // Пытаемся положить guard. `set` падает только если уже
            // лежит — это значит, init() уже был вызван; молча мирится.
            let _ = LOG_GUARD.set(guard);
            Some(
                fmt::layer()
                    .with_writer(non_blocking)
                    .with_ansi(false)
                    .with_target(true)
                    .with_thread_ids(false)
                    .with_thread_names(true)
                    .with_filter(EnvFilter::new(
                        // html5ever/markup5ever/selectors шумят `WARN node with
                        // weird namespace ...` при парсинге HTML readability'ем
                        // внутри tool `web` — глушим до error.
                        // synthos-fs-watcher / synthos-git-status / hyper-pool
                        // дают сотни тысяч DEBUG строк за час (file events,
                        // соединения keep-alive) и забивают файл; на post-mortem
                        // отладке они не нужны — оставляем info+.
                        "debug,wgpu_core=warn,wgpu_hal=warn,naga=warn,\
                         html5ever=error,markup5ever=error,selectors=error,\
                         synthos-fs-watcher=info,synthos-git-status=info,\
                         hyper_util::client::legacy::pool=info",
                    )),
            )
        }
        Err(e) => {
            // Не смогли открыть файл (permission denied, full disk и т.п.) —
            // продолжаем без файлового layer'а. stderr-only режим.
            file_layer_failed = Some(format!(
                "file logging disabled: {e} (target dir: {})",
                log_dir.display()
            ));
            None
        }
    };

    // Stderr layer: INFO по умолчанию, ANSI, можно переопределить через RUST_LOG.
    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true)
        .with_target(false)
        .with_thread_ids(false)
        .with_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                EnvFilter::new(
                    "info,wgpu_core=warn,wgpu_hal=warn,naga=warn,\
                     html5ever=error,markup5ever=error,selectors=error",
                )
            }),
        );

    let _ = tracing_log::LogTracer::init();

    let registry = tracing_subscriber::registry().with(stderr_layer);
    let _ = match file_layer {
        Some(file) => registry.with(file).try_init(),
        None => registry.try_init(),
    };

    if let Some(msg) = file_layer_failed {
        tracing::warn!("{msg}");
    } else {
        tracing::info!(
            log_dir = %log_dir.display(),
            "файловое логирование инициализировано (rolling daily)"
        );
    }
}

fn prepare_file_writer(
    log_dir: &std::path::Path,
) -> std::io::Result<(tracing_appender::non_blocking::NonBlocking, WorkerGuard)> {
    std::fs::create_dir_all(log_dir)?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("synthos")
        .filename_suffix("log")
        .build(log_dir)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let (nb, guard) = tracing_appender::non_blocking(appender);
    Ok((nb, guard))
}

/// Канонический путь директории логов. Не паникует — на отсутствие HOME
/// возвращает `./logs`. Реальный mkdir делается в [`prepare_file_writer`].
pub fn log_dir_path() -> PathBuf {
    if let Some(p) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(p).join("synthos/logs");
    }
    #[cfg(windows)]
    {
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(p).join("synthos/logs");
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/state/synthos/logs");
    }
    PathBuf::from("./synthos-logs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_dir_with_xdg_state_home() {
        let prev = std::env::var_os("XDG_STATE_HOME");
        // Используем temp-путь, чтобы не зависеть от хостовой машины.
        std::env::set_var("XDG_STATE_HOME", "/tmp/synthos-test-state");
        let p = log_dir_path();
        assert_eq!(p, PathBuf::from("/tmp/synthos-test-state/synthos/logs"));
        match prev {
            Some(v) => std::env::set_var("XDG_STATE_HOME", v),
            None => std::env::remove_var("XDG_STATE_HOME"),
        }
    }

    #[test]
    fn log_dir_falls_back_to_home_local_state() {
        let prev_xdg = std::env::var_os("XDG_STATE_HOME");
        let prev_home = std::env::var_os("HOME");
        std::env::remove_var("XDG_STATE_HOME");
        std::env::set_var("HOME", "/tmp/synthos-test-home");
        let p = log_dir_path();
        assert_eq!(
            p,
            PathBuf::from("/tmp/synthos-test-home/.local/state/synthos/logs")
        );
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_STATE_HOME", v),
            None => {}
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
