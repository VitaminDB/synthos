//! Терминалы, переживающие перезапуск synthos: их PTY держит процесс-хранитель
//! из syngui ([`syngui::widgets::visual::terminal::holder`]) — тот же бинарник,
//! запущенный как `synthos --pty-holder <сокет>`. Вкладка помнит ключ
//! терминала в хранителе; состав вкладок сохраняется в
//! `CodeSessionConfig::terminals`, и при запуске synthos подключается к живым
//! терминалам. Терминалы хранителя, которых нет в сохранённом составе
//! (конфиг потерян, вкладка была «отсоединена»), не теряются: они
//! возвращаются в сессию своего проекта — см. [`super::state::restore_terminals`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use syngui::widgets::visual::terminal::holder::{self, HeldInfo, HolderSpec};

/// Аргумент командной строки, которым запускается хранитель.
pub const HOLDER_ARG: &str = "--pty-holder";

/// Сокет хранителя: `$XDG_RUNTIME_DIR/synthos/pty-holder.sock`. Путь unix-
/// сокета ограничен 108 байтами — при слишком длинном `XDG_RUNTIME_DIR`
/// берётся короткий `/tmp/synthos-<uid>/pty-<хеш>.sock`.
pub fn socket_path() -> PathBuf {
    // SAFETY: getuid без побочных эффектов.
    let uid = unsafe { libc::getuid() };
    let tmp = std::env::temp_dir().join(format!("synthos-{uid}"));
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| tmp.clone());
    let path = base.join("synthos").join("pty-holder.sock");
    if path.as_os_str().len() < 100 {
        return path;
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    tmp.join(format!("pty-{:x}.sock", h.finish()))
}

/// Данные synthos у терминала в хранителе: по ним осиротевший терминал
/// возвращается в сессию своего проекта.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TerminalMeta {
    /// Корень проекта сессии (`None` — сессия-«Терминал» или без папки).
    pub folder: Option<String>,
    pub terminal_only: bool,
    pub title: String,
}

impl TerminalMeta {
    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn decode(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or_default()
    }
}

/// Новый уникальный ключ терминала.
pub fn new_key() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{nanos:x}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Команда запуска хранителя: этот же бинарник с [`HOLDER_ARG`]. После
/// обновления пакета `/proc/self/exe` указывает на «(deleted)» — берём путь
/// без этой приписки (новый бинарник на том же месте).
fn spawn_argv(socket: &Path) -> Vec<String> {
    let exe = std::env::current_exe()
        .map(|p| {
            let s = p.to_string_lossy().into_owned();
            s.strip_suffix(" (deleted)").map(str::to_string).unwrap_or(s)
        })
        .unwrap_or_else(|_| "synthos".into());
    vec![exe, HOLDER_ARG.into(), socket.to_string_lossy().into_owned()]
}

/// Описание подключения вкладки к хранителю. `create = false` — только к
/// уже живому терминалу (восстановление), иначе новый.
pub fn spec(key: &str, meta: &TerminalMeta, create: bool) -> HolderSpec {
    let socket = socket_path();
    HolderSpec {
        spawn: spawn_argv(&socket),
        socket,
        key: key.to_string(),
        meta: meta.encode(),
        create,
    }
}

/// Живые терминалы хранителя (хранитель не запущен — пусто).
pub fn list() -> Vec<HeldInfo> {
    match holder::list(&socket_path()) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "хранитель терминалов: список не получен");
            Vec::new()
        }
    }
}

/// Точка входа `synthos --pty-holder <сокет>`.
pub fn run(socket: &Path) -> std::io::Result<()> {
    holder::run_holder(socket)
}
