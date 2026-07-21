//! Body для NodeKind::SaveToFile — запись AudioStream в WAV-файл (PCM16).
//!
//! Layout:
//!   [in] [TextField path] [Record/Stop toggle] [status row]
//!
//! ## Поток данных
//!
//! 1. Executor (`SaveToFileExec::evaluate`) при detection нового
//!    `PortValue::AudioStream(s)` забирает `s.take_receiver()` и кладёт
//!    в `runtime.pending_rx` вместе с `(sample_rate, channels)`.
//!    Receiver перехватывается ДАЖЕ если запись ещё не стартована —
//!    иначе мы теряем upstream при single-sub'е.
//! 2. UI (Record-клик) забирает `pending_rx`, открывает
//!    [`syngui::audio::WavStreamWriter`] по пути из TextField и спавнит
//!    worker `recv → write_chunk`. Завершение через `cancel`-флаг.
//! 3. Stop-клик ставит `cancel=true` и дроп worker'а — finalize'ит файл,
//!    обновляет SaveStatus.
//!
//! Re-Record после Stop: нужен новый detection upstream'а (отсоединить и
//! заново подключить провод), либо upstream продолжает гонять — тогда
//! receiver уже потреблён предыдущим worker'ом, и пользователю надо
//! пересоединить (стандартное поведение single-sub).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use syngui::audio::WavStreamWriter;
use syngui::core::sync::Mutex;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::input::TextField;
use syngui::widgets::{DecoratedBox, Reactive, Row, ToolButton};

use crate::icons::{MI_FOLDER_OPEN, MI_SAVE, MI_STOP};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::types::{NodeInstance, NodeRuntime, PortValue, PrevSource, SaveStatus};

/// Executor для SaveToFile.
pub struct SaveToFileExec;

impl NodeExecutor for SaveToFileExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let in_pv = ctx.read_input("in");
        let new_kind = match &in_pv {
            PortValue::AudioStream(s) => PrevSource::Stream(Arc::as_ptr(s) as usize),
            PortValue::Audio(buf) => PrevSource::Buffer(Arc::as_ptr(buf) as usize),
            _ => PrevSource::None,
        };

        if let Ok(mut g) = ctx.runtime().lock() {
            if let NodeRuntime::SaveToFile {
                last_input_kind,
                pending_rx,
                has_upstream,
                worker,
                is_writing,
                status,
                cancel,
                ..
            } = &mut *g
            {
                if new_kind != *last_input_kind {
                    *last_input_kind = new_kind;
                    // Отвязка от старого источника: если шла запись,
                    // останавливаем worker (cooperative cancel + drop handle).
                    if let Some(h) = worker.take() {
                        cancel.store(true, Ordering::Relaxed);
                        drop(h);
                    }
                    is_writing.set(false);
                    if !matches!(
                        status.get_untracked(),
                        SaveStatus::Saved(_) | SaveStatus::Error(_)
                    ) {
                        status.set(SaveStatus::Idle);
                    }
                    // Очищаем старый pending_rx и забираем новый, если есть.
                    if let Ok(mut g_rx) = pending_rx.lock() {
                        *g_rx = None;
                    }
                    match &in_pv {
                        PortValue::AudioStream(s) => {
                            if let Some(rx) = s.take_receiver() {
                                if let Ok(mut g_rx) = pending_rx.lock() {
                                    *g_rx = Some((rx, s.sample_rate, s.channels));
                                }
                                has_upstream.set(true);
                            } else {
                                has_upstream.set(false);
                            }
                        }
                        PortValue::Audio(buf) => {
                            // Buffer-источник: разбиваем PCM на чанки и
                            // пушим в mpsc-channel из gateway-треда. Writer-loop
                            // работает без изменений — он видит обычный
                            // Receiver<Vec<f32>>, recv'ит до RecvError.
                            let (tx, rx) = mpsc::channel::<Vec<f32>>();
                            let buf_clone = buf.clone();
                            let sr = buf.sample_rate;
                            let ch = buf.channels;
                            thread::Builder::new()
                                .name("synthos-save-buffer-gateway".into())
                                .spawn(move || {
                                    // Чанк ≈ 100 ms аудио — баланс между числом
                                    // recv'ов и накладными в writer_loop.
                                    let frames_per_chunk = (sr as usize / 10).max(1);
                                    let samples_per_chunk = frames_per_chunk * ch as usize;
                                    for chunk in buf_clone.pcm.chunks(samples_per_chunk.max(1)) {
                                        if tx.send(chunk.to_vec()).is_err() {
                                            return;
                                        }
                                    }
                                    // Drop tx → recv даёт Err → writer_loop finalize.
                                })
                                .ok();
                            if let Ok(mut g_rx) = pending_rx.lock() {
                                *g_rx = Some((rx, sr, ch));
                            }
                            has_upstream.set(true);
                        }
                        _ => {
                            has_upstream.set(false);
                        }
                    }
                }
            }
        }
        // SaveToFile — sink, output port у неё нет.
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();

    let (path_sig, is_writing, written_secs, written_bytes, status_sig, has_upstream) =
        match runtime.lock() {
            Ok(g) => match &*g {
                NodeRuntime::SaveToFile {
                    path,
                    is_writing,
                    written_seconds,
                    written_bytes,
                    status,
                    has_upstream,
                    ..
                } => (
                    *path,
                    *is_writing,
                    *written_seconds,
                    *written_bytes,
                    *status,
                    *has_upstream,
                ),
                _ => return error_widget("SaveToFile: некорректный runtime"),
            },
            Err(_) => return error_widget("SaveToFile: lock error"),
        };

    let path_input = TextField::new()
        .text(path_sig.get_untracked())
        .placeholder("~/Downloads/synthos-….wav")
        .width(280.0)
        .on_change(move |s| {
            path_sig.set(s.to_string());
        });

    let browse_btn = ToolButton::new(MI_FOLDER_OPEN)
        .tooltip("Выбрать файл…")
        .on_click(move || {
            if let Some(p) = pick_save_path(&path_sig.get_untracked()) {
                path_sig.set(p);
            }
        })
        .class("audio-node-transport-btn audio-node-stop");

    let runtime_btn = runtime.clone();
    let toggle_btn = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let writing = is_writing.get();
        let upstream_ok = has_upstream.get();
        let runtime_btn = runtime_btn.clone();
        let (icon, tooltip, class) = if writing {
            (
                MI_STOP,
                "Остановить запись",
                "audio-node-transport-btn audio-node-record-active",
            )
        } else if !upstream_ok {
            (
                MI_SAVE,
                "Подключите AudioStream",
                "audio-node-transport-btn audio-node-stop",
            )
        } else {
            (
                MI_SAVE,
                "Начать запись",
                "audio-node-transport-btn audio-node-record",
            )
        };
        let btn = ToolButton::new(icon)
            .tooltip(tooltip)
            .on_click(move || {
                if is_writing.get_untracked() {
                    stop_writing(&runtime_btn);
                } else if has_upstream.get_untracked() {
                    start_writing(
                        &runtime_btn,
                        path_sig,
                        is_writing,
                        written_secs,
                        written_bytes,
                        status_sig,
                        has_upstream,
                    );
                }
            })
            .class(class);
        vec![Box::new(btn)]
    });

    let status_label = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let st = status_sig.get();
        let secs = written_secs.get();
        let bytes = written_bytes.get();
        let (txt, class) = match st {
            SaveStatus::Idle => {
                if has_upstream.get() {
                    (
                        "Готов".to_string(),
                        "save-node-status".to_string(),
                    )
                } else {
                    (
                        "Подключите источник".to_string(),
                        "save-node-status".to_string(),
                    )
                }
            }
            SaveStatus::Writing => (
                format!(
                    "Запись · {} · {:.1} КБ",
                    fmt_mmss(secs),
                    bytes as f64 / 1024.0
                ),
                "save-node-status save-node-status-active".to_string(),
            ),
            SaveStatus::Saved(p) => (
                format!(
                    "Сохранено · {} · {:.1} КБ",
                    p.file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| p.display().to_string()),
                    bytes as f64 / 1024.0
                ),
                "save-node-status save-node-status-saved".to_string(),
            ),
            SaveStatus::Error(msg) => (format!("Ошибка: {msg}"), "audio-node-error".to_string()),
        };
        vec![Box::new(Text::new(txt).class(class)) as Box<dyn Widget>]
    });

    let row = mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().child(path_input).class("save-node-path-host"),
                DecoratedBox::new().child(browse_btn).class("audio-node-slot-btn"),
                DecoratedBox::new().child(toggle_btn).class("audio-node-slot-btn"),
                DecoratedBox::new().child(status_label).class("save-node-status-host"),
            ]
    };

    Box::new(
        DecoratedBox::new()
            .child(row)
            .class("audio-node-body-host audio-save-host"),
    )
}

/// Системный save-dialog (rfd) с фильтром на WAV. `current` — текущий путь
/// из TextField, используется как стартовая директория и имя по умолчанию.
/// Возвращает выбранный путь как строку или None если пользователь отменил.
fn pick_save_path(current: &str) -> Option<String> {
    let mut dlg = rfd::FileDialog::new().add_filter("WAV", &["wav"]);
    let cur_path = std::path::PathBuf::from(current);
    if let Some(parent) = cur_path.parent() {
        if parent.is_dir() {
            dlg = dlg.set_directory(parent);
        }
    }
    if let Some(name) = cur_path.file_name().and_then(|s| s.to_str()) {
        if !name.is_empty() {
            dlg = dlg.set_file_name(name);
        }
    }
    let mut path = dlg.save_file()?;
    // Гарантируем .wav расширение, чтобы writer не словил mismatch.
    if path.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("wav")) != Some(true) {
        path.set_extension("wav");
    }
    Some(path.to_string_lossy().to_string())
}

fn error_widget(msg: &'static str) -> Box<dyn Widget> {
    Box::new(
        Padding::symmetric(10.0, 6.0)
            .child(Text::new(msg).class("audio-node-error")),
    )
}

fn fmt_mmss(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let cs = ((secs - s as f64) * 100.0) as u64;
    format!("{:02}:{:02}.{:02}", s / 60, s % 60, cs)
}

fn start_writing(
    runtime: &Arc<Mutex<NodeRuntime>>,
    path_sig: RwSignal<String>,
    is_writing: RwSignal<bool>,
    written_secs: RwSignal<f64>,
    written_bytes: RwSignal<u64>,
    status_sig: RwSignal<SaveStatus>,
    has_upstream: RwSignal<bool>,
) {
    // Захватываем receiver из pending_rx и подготавливаем cancel-флаг.
    let (rx, sample_rate, channels, cancel) = {
        let mut g = match runtime.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let NodeRuntime::SaveToFile {
            pending_rx,
            cancel,
            worker,
            ..
        } = &mut *g
        else {
            return;
        };
        if worker.is_some() {
            return;
        }
        let Some((rx, sr, ch)) = pending_rx
            .lock()
            .ok()
            .and_then(|mut g_rx| g_rx.take())
        else {
            return;
        };
        cancel.store(false, Ordering::Relaxed);
        (rx, sr, ch, cancel.clone())
    };

    let path = PathBuf::from(path_sig.get_untracked());

    // Сбрасываем счётчики.
    written_secs.set(0.0);
    written_bytes.set(0);

    // Открываем writer (CR file). Ошибка → status=Error, has_upstream=false
    // (receiver уже потреблён, нужен reconnect).
    let writer = match WavStreamWriter::open(&path, sample_rate, channels) {
        Ok(w) => w,
        Err(e) => {
            status_sig.set(SaveStatus::Error(format!("{e}")));
            has_upstream.set(false);
            return;
        }
    };

    is_writing.set(true);
    status_sig.set(SaveStatus::Writing);

    let h = thread::Builder::new()
        .name("synthos-save-to-file-worker".into())
        .spawn(move || {
            writer_loop(
                rx,
                writer,
                cancel,
                path,
                sample_rate,
                channels,
                written_secs,
                written_bytes,
                status_sig,
                is_writing,
                has_upstream,
            )
        })
        .ok();

    if let Ok(mut g) = runtime.lock() {
        if let NodeRuntime::SaveToFile { worker, .. } = &mut *g {
            *worker = h;
        }
    }
}

fn writer_loop(
    rx: mpsc::Receiver<Vec<f32>>,
    mut writer: WavStreamWriter,
    cancel: Arc<AtomicBool>,
    path: PathBuf,
    sample_rate: u32,
    channels: u16,
    written_secs: RwSignal<f64>,
    written_bytes: RwSignal<u64>,
    status_sig: RwSignal<SaveStatus>,
    is_writing: RwSignal<bool>,
    has_upstream: RwSignal<bool>,
) {
    let mut total_samples: u64 = 0;
    let bytes_per_sample = 2_u64; // PCM16
    let ch = channels.max(1) as u64;
    let sr = sample_rate.max(1);

    let mut error: Option<String> = None;
    while !cancel.load(Ordering::Relaxed) {
        // recv_timeout не используем — sender дропнется при отключении
        // upstream'а, что даст RecvError. Cancel-флаг проверяем перед каждым
        // recv, но для чистого Stop достаточно cancel.store(true) + drop.
        match rx.recv() {
            Ok(chunk) => {
                if let Err(e) = writer.write_chunk(&chunk) {
                    error = Some(e.to_string());
                    break;
                }
                total_samples += chunk.len() as u64;
                written_secs.set(total_samples as f64 / sr as f64 / ch as f64);
                written_bytes.set(total_samples * bytes_per_sample);
            }
            Err(_) => break,
        }
    }

    match writer.finalize() {
        Ok(()) => {
            if let Some(msg) = error {
                status_sig.set(SaveStatus::Error(msg));
            } else {
                status_sig.set(SaveStatus::Saved(path));
            }
        }
        Err(e) => status_sig.set(SaveStatus::Error(e.to_string())),
    }
    is_writing.set(false);
    // Receiver исчерпан/закрыт — повторно записать с тем же upstream нельзя.
    has_upstream.set(false);
}

fn stop_writing(runtime: &Arc<Mutex<NodeRuntime>>) {
    if let Ok(mut g) = runtime.lock() {
        if let NodeRuntime::SaveToFile { cancel, worker, .. } = &mut *g {
            cancel.store(true, Ordering::Relaxed);
            // Drop handle (поток сам finalize'нет файл и обновит status).
            if let Some(h) = worker.take() {
                drop(h);
            }
        }
    }
}
