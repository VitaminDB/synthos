//! Движок нейросетей synaptix для всей программы.
//!
//! Чат, генерация, нодовый редактор, база знаний, обучение говорят с движком
//! одним клиентом (`synaptixd::Client`). Сам движок — либо системная служба
//! `synaptixd`, если она запущена (её модели общие со всеми программами
//! synshell), либо тот же сервер из крейта `synaptixd::server`, поднятый
//! внутри процесса synthos: служба не нужна, программа работает сама.
//!
//! `SYNTHOS_ENGINE=service` — только системная служба (без неё нейросетей
//! нет), `SYNTHOS_ENGINE=embedded` — всегда свой движок, даже при запущенной
//! службе. Без переменной — служба, если отвечает, иначе свой.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Где исполняются модели.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Engine {
    /// Системная служба synaptixd.
    Service,
    /// Свой движок в процессе synthos на этом сокете.
    Embedded(PathBuf),
    /// Движка нет: служба не отвечает, а свой не собран или не поднялся.
    None(String),
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// Выбрать и при необходимости запустить движок. Вызывать на старте, до
/// первых обращений к моделям; повторные вызовы возвращают выбор первого.
pub fn ensure() -> &'static Engine {
    ENGINE.get_or_init(|| {
        let e = pick();
        match &e {
            Engine::Service => tracing::info!(socket = %synaptixd::socket_path().display(), "движок: системная служба synaptixd"),
            Engine::Embedded(s) => tracing::info!(socket = %s.display(), "движок: встроенный (без службы synaptixd)"),
            Engine::None(why) => tracing::warn!("движка нейросетей нет: {why}"),
        }
        e
    })
}

/// Текущий движок (после [`ensure`]).
pub fn current() -> Option<&'static Engine> {
    ENGINE.get()
}

fn pick() -> Engine {
    let mode = std::env::var("SYNTHOS_ENGINE").unwrap_or_default();
    let service_up = || synaptixd::Client::connect_to(&synaptixd::socket_path()).is_ok();
    match mode.as_str() {
        "service" => {
            if service_up() {
                Engine::Service
            } else {
                Engine::None(format!("служба synaptixd не отвечает ({})", synaptixd::socket_path().display()))
            }
        }
        "embedded" => start_embedded(),
        _ if service_up() => Engine::Service,
        _ => start_embedded(),
    }
}

#[cfg(feature = "engine")]
fn start_embedded() -> Engine {
    let socket = runtime_dir().join(format!("synthos/synaptixd-{}.sock", std::process::id()));
    clean_stale(socket.parent());
    match synaptixd::server::embedded(&socket, &state_dir()) {
        Ok(()) => {
            synaptixd::set_socket_path(socket.clone());
            Engine::Embedded(socket)
        }
        Err(e) => Engine::None(format!("встроенный движок не запустился: {e:#}")),
    }
}

#[cfg(not(feature = "engine"))]
fn start_embedded() -> Engine {
    Engine::None("служба synaptixd не запущена, а synthos собран без встроенного движка (фича `engine`)".into())
}

/// Сокет каждого процесса свой (`synaptixd-<pid>.sock`); от завершившихся
/// остаются файлы — убрать их, чтобы каталог не рос.
#[cfg(feature = "engine")]
fn clean_stale(dir: Option<&std::path::Path>) {
    let Some(rd) = dir.and_then(|d| std::fs::read_dir(d).ok()) else { return };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.strip_prefix("synaptixd-")).and_then(|n| n.strip_suffix(".sock")) else { continue };
        if !std::path::Path::new("/proc").join(pid).exists() {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(feature = "engine")]
fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("synthos-{}", unsafe { libc::geteuid() })))
}

/// Состояние своего движка (библиотеки моделей, выбор моделей, голоса,
/// задания обучения) — рядом с логами synthos.
#[cfg(feature = "engine")]
fn state_dir() -> PathBuf {
    crate::logging::log_dir_path().parent().map(|p| p.join("synaptixd")).unwrap_or_else(|| PathBuf::from("./synaptixd"))
}
