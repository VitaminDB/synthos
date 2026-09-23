//! Инициализация tracing: stderr (для разработчика) + rolling file
//! appender (для post-mortem отладки на машине пользователя).
//!
//! Пути логов:
//! - Linux:   `$XDG_STATE_HOME/synthos/logs/` (по умолчанию `~/.local/state/synthos/logs/`).
//! - macOS:   `~/.local/state/synthos/logs/` (тот же путь, не лезем в `~/Library/Logs/`).
//! - Windows: `%LOCALAPPDATA%\synthos\logs` или `<HOME>\.local\state\synthos\logs`.
//!
//! Файлы ротируются по дням: `synthos.YYYY-MM-DD.log`. `tracing-appender`
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
                    .with_filter(EnvFilter::new(file_filter())),
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
        prune_old_logs(&log_dir, RETENTION_DAYS);
    }
}

/// Директивы фильтра файлового layer'а: `debug` для нашего кода, шумные
/// сторонние крейты приглушены.
///
/// Директивы матчатся по **target**, а не по имени потока: `synthos-fs-watcher`
/// (имя треда) не совпадало ни с чем, и фильтр молча не работал — отсюда логи
/// по 500 МБ в день. Дефис в target'е EnvFilter не принимает, поэтому
/// `code-editor` глушим по общему префиксу.
///
/// Кто и чем шумит:
/// - `h2` — по кадру на каждый чтение/запись HTTP/2: 6.4 млн строк (99.9 %
///   файла) за сутки работы tool `web`;
/// - `html5ever`/`markup5ever`/`selectors` — `WARN node with weird namespace`
///   на каждый тег при парсинге HTML readability'ем;
/// - `code-editor` (fs_watcher/git_status) и `hyper-pool` — сотни тысяч строк
///   в час на file events и keep-alive;
/// - `notify` — inotify-события файловых вотчеров;
/// - `symphonia_*` — probe формата на каждом открытии wav (плеер, превью нод).
fn file_filter() -> String {
    const QUIET_INFO: &[&str] = &[
        "code-editor",
        "hyper_util::client::legacy::pool",
        "h2",
        "rustls",
    ];
    const QUIET_WARN: &[&str] = &[
        "wgpu_core",
        "wgpu_hal",
        "naga",
        "notify",
        "symphonia",
        "symphonia_core",
        "symphonia_bundle_flac",
        "symphonia_bundle_mp3",
        "symphonia_codec_aac",
        "symphonia_codec_pcm",
        "symphonia_codec_vorbis",
        "symphonia_format_isomp4",
        "symphonia_format_ogg",
        "symphonia_format_riff",
    ];
    const QUIET_ERROR: &[&str] = &["html5ever", "markup5ever", "selectors"];

    let mut directives = vec!["debug".to_string()];
    for (targets, level) in
        [(QUIET_INFO, "info"), (QUIET_WARN, "warn"), (QUIET_ERROR, "error")]
    {
        directives.extend(targets.iter().map(|t| format!("{t}={level}")));
    }
    directives.join(",")
}

/// Сколько дней держим старые `synthos.YYYY-MM-DD.log`.
const RETENTION_DAYS: u64 = 30;

/// Удаляет ротированные логи старше `days` дней. `tracing-appender` их
/// только создаёт и никогда не убирает — за три месяца каталог набирал
/// десятки гигабайт (одна сессия с DEBUG'ом h2 давала ~1 ГБ в сутки).
///
/// Трогает строго `synthos.*.log` в каталоге логов, не рекурсивно и не
/// по симлинкам; текущий файл (сегодняшний) под порог не попадает.
/// Ошибки удаления игнорируются: чистка не должна мешать старту.
fn prune_old_logs(log_dir: &std::path::Path, days: u64) {
    let Ok(entries) = std::fs::read_dir(log_dir) else { return };
    let cutoff = match std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(days * 24 * 60 * 60))
    {
        Some(t) => t,
        None => return,
    };
    let (mut removed, mut freed) = (0usize, 0u64);
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.starts_with("synthos.") || !name.ends_with(".log") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else { continue };
        if modified >= cutoff {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
            freed += meta.len();
        }
    }
    if removed > 0 {
        tracing::info!(
            removed,
            freed_mb = freed / (1024 * 1024),
            days,
            "старые логи удалены"
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

    /// Оба теста крутят одни и те же env-переменные, а cargo гоняет тесты
    /// параллельными потоками — без сериализации они флачат, перетирая
    /// XDG_STATE_HOME/HOME друг у друга посреди проверки.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Фильтр парсится целиком: EnvFilter молча отбрасывает невалидные
    /// директивы (`parse_lossy`), и опечатка в имени крейта означала бы
    /// «глушение не работает» без единого признака.
    #[test]
    fn file_filter_parses_every_directive() {
        let filter = file_filter();
        let strict = tracing_subscriber::filter::EnvFilter::builder()
            .parse(&filter)
            .unwrap_or_else(|e| panic!("фильтр не разобрался: {e}\n{filter}"));
        let printed = strict.to_string();
        for target in ["notify", "symphonia_core", "h2", "wgpu_core"] {
            assert!(printed.contains(target), "{target} потерялся: {printed}");
        }
    }

    /// Ретеншн сносит только старые `synthos.*.log` и не трогает свежие
    /// и чужие файлы.
    #[test]
    fn prune_removes_only_old_synthos_logs() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("synthos.2020-01-01.log");
        let fresh = dir.path().join("synthos.2026-08-27.log");
        let alien = dir.path().join("other.2020-01-01.log");
        for p in [&old, &fresh, &alien] {
            std::fs::write(p, b"x").unwrap();
        }
        // Состарим два файла на 60 дней: удалиться должен только synthos.*.
        let ancient = std::time::SystemTime::now()
            - std::time::Duration::from_secs(60 * 24 * 60 * 60);
        for p in [&old, &alien] {
            let f = std::fs::File::options().write(true).open(p).unwrap();
            f.set_modified(ancient).unwrap();
        }

        prune_old_logs(dir.path(), 30);

        assert!(!old.exists(), "старый лог должен быть удалён");
        assert!(fresh.exists(), "свежий лог трогать нельзя");
        assert!(alien.exists(), "чужие файлы трогать нельзя");
    }

    #[test]
    fn log_dir_with_xdg_state_home() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_xdg = std::env::var_os("XDG_STATE_HOME");
        let prev_home = std::env::var_os("HOME");
        std::env::remove_var("XDG_STATE_HOME");
        std::env::set_var("HOME", "/tmp/synthos-test-home");
        let p = log_dir_path();
        assert_eq!(
            p,
            PathBuf::from("/tmp/synthos-test-home/.local/state/synthos/logs")
        );
        if let Some(v) = prev_xdg { std::env::set_var("XDG_STATE_HOME", v) }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
