//! Обёртка над дочерним процессом `llama-server`.
//!
//! Запускается через `std::process::Command`, stderr/stdout читаются в двух
//! фоновых потоках построчно и пушатся в реактивный сигнал через
//! `async_runtime::run_on_main_thread`. Это даёт UI автоматический redraw
//! без polling’а.
//!
//! Один экземпляр живёт всё время работы приложения: `AppCtx.llama: Arc<…>`.
//! `Drop` гарантированно убивает процесс, если приложение закрыли, пока
//! сервер ещё бежал.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use syngui::async_runtime::run_on_main_thread;
use syngui::core::sync::Mutex;
use syngui::signal::{use_signal, RwSignal};

use crate::config::{ModelConfig, ParamValue};
use crate::pages::settings::models::llama_params::{self, LlamaParam, ParamKind, LLAMA_PARAMS};

/// Максимум строк в буфере логов (старое вытесняется head-drain).
const MAX_LOG_LINES: usize = 5000;

/// Параметры llama-server, которые мы подставляем отдельно:
/// `--ctx-size` уже идёт из `ModelConfig.ctx_size`, путь к модели / mmproj —
/// из `ModelConfig.model_path` / `mmproj_path`, а `--host`/`--port` —
/// из `GeneralSettingsSnapshot`. Если кто-то вручную добавил такие ключи
/// в `active_params` — тихо пропускаем в CLI-билдере.
const SKIP_IN_ACTIVE: &[&str] = &["ctx_size", "model", "mmproj", "host", "port"];

// ─────────────────────────────────────────────────────────────────────────────
// Публичные типы
// ─────────────────────────────────────────────────────────────────────────────

/// Тип процесса llama-server.
///
/// Раньше существовал вариант `Audio` для запуска ASR-сервера (whisper.cpp /
/// llama-server `/v1/audio/transcriptions`). Сейчас ASR работает локально
/// через [`synaptix::facade::asr`] — внешний процесс не нужен. Enum сохранён
/// одного варианта для обратной совместимости вызовов; будет удалён, когда
/// отпадёт надобность.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessKind {
    /// Текстовая чат-модель.
    Chat,
}

impl ProcessKind {
    fn log_prefix(self) -> &'static str {
        match self {
            ProcessKind::Chat => "chat",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProcessStatus {
    Stopped = 0,
    Starting = 1,
    Running = 2,
    Error = 3,
}

impl ProcessStatus {
    pub fn label(self) -> &'static str {
        match self {
            ProcessStatus::Stopped => "Остановлен",
            ProcessStatus::Starting => "Запуск…",
            ProcessStatus::Running => "Работает",
            ProcessStatus::Error => "Ошибка",
        }
    }

    /// Имя состояния, добавляемое к имени базового класса для получения
    /// независимого CSS-класса (напр., «llama-status-pill-running»).
    /// Одиночные селекторы надёжнее compound-селекторов в MSS-движке.
    pub fn css_modifier(self) -> &'static str {
        match self {
            ProcessStatus::Stopped => "stopped",
            ProcessStatus::Starting => "starting",
            ProcessStatus::Running => "running",
            ProcessStatus::Error => "error",
        }
    }
}

impl From<u8> for ProcessStatus {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::Starting,
            2 => Self::Running,
            3 => Self::Error,
            _ => Self::Stopped,
        }
    }
}

/// Снимок глобальных настроек, необходимых для запуска.
/// Берётся из `AppCtx.general` через `get_untracked()` в момент нажатия «Запустить».
#[derive(Debug, Clone)]
pub struct GeneralSnapshot {
    pub server_path: String,
    pub host: String,
    pub port: u16,
}

/// Управление процессом `llama-server`.
///
/// Потокобезопасен за счёт `Arc<Mutex<_>>` внутри. UI пользуется только
/// `status_signal` / `logs_signal` — это `Copy`-сигналы.
pub struct LlamaProcess {
    child: Arc<Mutex<Option<Child>>>,
    status: Arc<AtomicU8>,
    pub status_signal: RwSignal<ProcessStatus>,
    pub logs_signal: RwSignal<Vec<String>>,
    /// Тип процесса (chat/audio) — для префиксов логов и выбора CLI-билдера.
    pub kind: ProcessKind,
}

impl LlamaProcess {
    /// Создаёт новый процесс с типом по умолчанию (Chat).
    pub fn new() -> Self {
        Self::with_kind(ProcessKind::Chat)
    }

    /// Создаёт процесс заданного типа.
    pub fn with_kind(kind: ProcessKind) -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            status: Arc::new(AtomicU8::new(ProcessStatus::Stopped as u8)),
            status_signal: use_signal(ProcessStatus::Stopped),
            logs_signal: use_signal(Vec::new()),
            kind,
        }
    }

    pub fn status(&self) -> ProcessStatus {
        ProcessStatus::from(self.status.load(Ordering::Relaxed))
    }

    pub fn is_running(&self) -> bool {
        matches!(self.status(), ProcessStatus::Running | ProcessStatus::Starting)
    }

    /// Очистить буфер логов. Вызывается из UI («Очистить лог»).
    pub fn clear_logs(&self) {
        self.logs_signal.set(Vec::new());
    }

    /// Собирает CLI-аргументы согласно пресету. Публично — пригодится
    /// странице «Поддержка» и тестам.
    pub fn build_args(general: &GeneralSnapshot, model: &ModelConfig) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();

        // Пути
        if !model.model_path.trim().is_empty() {
            args.push("--model".into());
            args.push(model.model_path.clone());
        }
        if !model.mmproj_path.trim().is_empty() {
            args.push("--mmproj".into());
            args.push(model.mmproj_path.clone());
        }

        // ctx_size — всегда из модели
        args.push("--ctx-size".into());
        args.push(model.ctx_size.to_string());

        // Активные параметры
        for ap in &model.active_params {
            if SKIP_IN_ACTIVE.contains(&ap.key.as_str()) {
                continue;
            }
            let Some(param) = llama_params::by_key(&ap.key) else {
                continue;
            };
            push_param(&mut args, param, &ap.value);
        }

        // host/port — в конце
        args.push("--host".into());
        args.push(general.host.clone());
        args.push("--port".into());
        args.push(general.port.to_string());

        args
    }

    /// Запуск llama-server. Ошибка — текст для Snackbar.
    pub fn start(
        &self,
        general: GeneralSnapshot,
        model: ModelConfig,
    ) -> Result<(), String> {
        if self.is_running() {
            return Err("llama-server уже запущен".into());
        }
        let binary = if general.server_path.trim().is_empty() {
            "llama-server".to_string()
        } else {
            general.server_path.clone()
        };
        let args = Self::build_args(&general, &model);
        self.spawn_with_args(binary, args)
    }

    /// Общий код спауна процесса + чтения stdout/stderr.
    fn spawn_with_args(&self, binary: String, args: Vec<String>) -> Result<(), String> {
        let prefix = self.kind.log_prefix();

        self.logs_signal.set(Vec::new());
        self.push_log(format!("[{prefix}] $ {} {}", binary, args.join(" ")));

        self.set_status(ProcessStatus::Starting);

        let mut cmd = Command::new(&binary);
        cmd.args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            self.set_status(ProcessStatus::Error);
            let msg = format!("Не удалось запустить {binary}: {e}");
            self.push_log(msg.clone());
            msg
        })?;

        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        *self.child.lock().unwrap() = Some(child);

        // stderr — основной источник логов llama-server
        if let Some(stderr) = stderr_pipe {
            let logs = self.logs_signal;
            let status_atomic = self.status.clone();
            let status_signal = self.status_signal;
            let child_handle = self.child.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    match line {
                        Ok(l) => {
                            let is_ready = l.contains("server is listening");
                            push_log_from_thread(logs, l);
                            if is_ready {
                                status_atomic.store(ProcessStatus::Running as u8, Ordering::Relaxed);
                                run_on_main_thread(move || {
                                    status_signal.set(ProcessStatus::Running);
                                });
                            }
                        }
                        Err(_) => break,
                    }
                }
                // stderr закрылся — завершаем процесс, если ещё не остановлен
                let current = ProcessStatus::from(status_atomic.load(Ordering::Relaxed));
                if current == ProcessStatus::Starting {
                    status_atomic.store(ProcessStatus::Error as u8, Ordering::Relaxed);
                    run_on_main_thread(move || {
                        status_signal.set(ProcessStatus::Error);
                    });
                    push_log_from_thread(logs, "[!] Процесс завершился до готовности".into());
                } else if current != ProcessStatus::Stopped {
                    status_atomic.store(ProcessStatus::Stopped as u8, Ordering::Relaxed);
                    run_on_main_thread(move || {
                        status_signal.set(ProcessStatus::Stopped);
                    });
                    push_log_from_thread(logs, "[*] llama-server остановлен".into());
                }
                // Освобождаем handle — wait() съедает zombie, если kill() уже был.
                if let Ok(mut guard) = child_handle.lock() {
                    if let Some(mut c) = guard.take() {
                        let _ = c.wait();
                    }
                }
            });
        }

        // stdout — у llama-server обычно пусто, но на всякий случай тоже читаем
        if let Some(stdout) = stdout_pipe {
            let logs = self.logs_signal;
            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().flatten() {
                    push_log_from_thread(logs, line);
                }
            });
        }

        Ok(())
    }

    /// Остановить сервер. Идемпотентно.
    pub fn stop(&self) {
        let child = self.child.lock().unwrap().take();
        if let Some(mut c) = child {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.set_status(ProcessStatus::Stopped);
        self.push_log("[*] Сервер остановлен пользователем".into());
    }

    fn set_status(&self, s: ProcessStatus) {
        self.status.store(s as u8, Ordering::Relaxed);
        let signal = self.status_signal;
        run_on_main_thread(move || signal.set(s));
    }

    /// Пуш строки логов из главного потока (без `run_on_main_thread`).
    fn push_log(&self, line: String) {
        push_into_signal(self.logs_signal, line);
    }
}

impl Default for LlamaProcess {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LlamaProcess {
    fn drop(&mut self) {
        // Приложение закрывается или ctx дропается — не оставляем zombie.
        let child = self.child.lock().unwrap().take();
        if let Some(mut c) = child {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CLI-сборка отдельных параметров
// ─────────────────────────────────────────────────────────────────────────────

fn push_param(args: &mut Vec<String>, param: &LlamaParam, value: &ParamValue) {
    match (&param.kind, value) {
        (ParamKind::Int { .. }, ParamValue::Int(v)) => {
            args.push(param.cli.into());
            args.push(v.to_string());
        }
        (ParamKind::Float { decimals, .. }, ParamValue::Float(v)) => {
            args.push(param.cli.into());
            args.push(format!("{:.*}", *decimals as usize, v));
        }
        (ParamKind::Bool { .. }, ParamValue::Bool(true)) => {
            args.push(param.cli.into());
        }
        (ParamKind::Bool { .. }, ParamValue::Bool(false)) => {
            // выключенные bool не передаём (default — false); если у
            // параметра есть алиас вида --no-xxx, пользователь может выбрать
            // его как отдельный ключ.
        }
        (ParamKind::Text { .. }, ParamValue::Text(s)) if !s.is_empty() => {
            args.push(param.cli.into());
            args.push(s.clone());
        }
        (ParamKind::Text { .. }, ParamValue::Text(_)) => {
            // пустая строка — нет смысла передавать
        }
        (ParamKind::Enum { .. }, ParamValue::Enum(s)) if !s.is_empty() => {
            args.push(param.cli.into());
            args.push(s.clone());
        }
        (ParamKind::Enum { .. }, ParamValue::Enum(_)) => {}
        // Несоответствие типа — пропускаем (конфиг сломан); не паникуем.
        _ => {}
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Логи: head-drain и мост через run_on_main_thread
// ─────────────────────────────────────────────────────────────────────────────

fn push_log_from_thread(signal: RwSignal<Vec<String>>, line: String) {
    run_on_main_thread(move || push_into_signal(signal, line));
}

fn push_into_signal(signal: RwSignal<Vec<String>>, line: String) {
    signal.update(|v| {
        if v.len() >= MAX_LOG_LINES {
            let drop_n = v.len() + 1 - MAX_LOG_LINES;
            v.drain(..drop_n);
        }
        v.push(line);
    });
}

// Тривиальное предупреждение: LLAMA_PARAMS импортирован, но используется
// опосредованно через by_key — пометка на будущее, если появится код,
// читающий набор целиком.
#[allow(dead_code)]
fn _params_count() -> usize {
    LLAMA_PARAMS.len()
}
