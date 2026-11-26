//! Движок нейросетей для всей программы — системная служба synaptixd.
//!
//! Чат, генерация, база знаний и голосовой ввод говорят с ней одним
//! клиентом (`synaptixd::Client`); модели службы общие со всеми программами
//! synshell. Своего движка у synthos нет: без службы нейросети недоступны,
//! остальное (редактор кода, терминалы, заметки) работает.

use std::sync::OnceLock;

/// Где исполняются модели.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Engine {
    /// Системная служба synaptixd.
    Service,
    /// Служба не отвечает на старте (её могут запустить позже: клиенты
    /// подключаются заново на каждом запросе).
    None(String),
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// Проверить службу на старте. Повторные вызовы возвращают первый ответ.
pub fn ensure() -> &'static Engine {
    ENGINE.get_or_init(|| {
        let e = if synaptixd::Client::connect().is_ok() {
            Engine::Service
        } else {
            Engine::None(format!("служба synaptixd не отвечает ({})", synaptixd::socket_path().display()))
        };
        match &e {
            Engine::Service => tracing::info!(socket = %synaptixd::socket_path().display(), "движок: системная служба synaptixd"),
            Engine::None(why) => tracing::warn!("движка нейросетей нет: {why}"),
        }
        e
    })
}

/// Ответ [`ensure`] (после него).
pub fn current() -> Option<&'static Engine> {
    ENGINE.get()
}
