//! Body для NodeKind::AudioFile.
//!
//! Layout — горизонтальный, в стилистике Demo Node:
//!   [Open] [icon+filename+meta] [waveform-strip]
//!
//! Декодинг через `super::decode::decode_file` (symphonia) — поддерживается
//! WAV / MP3 / FLAC / OGG / AAC / M4A. Декодинг в фоновом потоке, результат
//! публикуется в `runtime.buffer` RwSignal.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use syngui::audio::AudioBuffer;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::visual::StaticWaveform;
use syngui::widgets::{Column, DecoratedBox, Reactive, Row, ToolButton};

use crate::icons::MI_FOLDER_OPEN;

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::types::{NodeInstance, NodeRuntime, PortValue};
use super::decode;

/// Executor для AudioFile: копирует загруженный буфер из `runtime.buffer`
/// в output port `out`. Загрузка PCM делается асинхронно из body builder'а
/// (см. `spawn_load`); сюда `evaluate` приходит уже после `RwSignal::set`,
/// так что `track=true` обеспечит реактивный пересчёт через `create_effect`.
pub struct AudioFileExec;

impl NodeExecutor for AudioFileExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let track = ctx.track;
        let pv = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::AudioFile { buffer, .. } => {
                    let b = if track { buffer.get() } else { buffer.get_untracked() };
                    b.map(PortValue::Audio).unwrap_or(PortValue::Empty)
                }
                _ => PortValue::Empty,
            },
            Err(_) => PortValue::Empty,
        };
        ctx.write_output("out", pv);
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();

    let (buffer_sig, path_sig, error_sig) = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::AudioFile { buffer, loaded_path, load_error } => {
                (*buffer, *loaded_path, *load_error)
            }
            _ => return error_widget("AudioFile: некорректный runtime"),
        },
        Err(_) => return error_widget("AudioFile: lock error"),
    };

    let open_btn = ToolButton::new(MI_FOLDER_OPEN)
        .tooltip("Открыть аудио-файл")
        .on_click(move || {
            let dlg = rfd::FileDialog::new()
                .add_filter("Аудио", &["wav", "mp3", "flac", "ogg", "m4a", "aac"])
                .add_filter("Все файлы", &["*"])
                .set_title("Выберите аудио-файл");
            let Some(path) = dlg.pick_file() else { return; };
            path_sig.set(Some(path.clone()));
            error_sig.set(None);
            buffer_sig.set(None);
            spawn_load(path, buffer_sig, error_sig);
        })
        .class("audio-node-open-btn");

    let info_panel = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let path_opt = path_sig.get();
        let buffer_opt = buffer_sig.get();
        let error_opt = error_sig.get();

        let mut col = Column::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Start);

        if let Some(err) = error_opt {
            col = col.child(Text::new(format!("Ошибка: {err}")).class("audio-node-error"));
        } else if let Some(path) = path_opt {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            col = col.child(Text::new(name).class("audio-node-filename"));
            if let Some(buf) = buffer_opt {
                let dur = buf.duration_seconds();
                let layout = if buf.channels >= 2 { "stereo" } else { "mono" };
                col = col.child(
                    Text::new(format!(
                        "{:.1} c · {} Hz · {}",
                        dur, buf.sample_rate, layout
                    ))
                    .class("audio-node-meta"),
                );
            } else {
                col = col.child(Text::new("Загрузка…").class("audio-node-meta"));
            }
        } else {
            col = col.child(Text::new("Файл не выбран").class("audio-node-empty"));
        }

        vec![Box::new(col) as Box<dyn Widget>]
    });

    let waveform = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let buf = buffer_sig.get();
        let widget: Box<dyn Widget> = match buf {
            Some(b) => Box::new(
                StaticWaveform::new()
                    .pcm(Some(b))
                    .height(40.0)
                    .class("audio-node-waveform"),
            ),
            None => Box::new(
                DecoratedBox::new()
                    .child(
                        Center::new().child(
                            Text::new("—").class("audio-node-waveform-placeholder"),
                        ),
                    )
                    .class("audio-node-waveform-empty"),
            ),
        };
        vec![widget]
    });

    let row = mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                open_btn,
                DecoratedBox::new().child(info_panel).class("audio-node-info"),
                DecoratedBox::new().child(waveform).class("audio-node-waveform-host"),
            ]
    };

    Box::new(
        DecoratedBox::new()
            .child(row)
            .class("audio-node-body-host audio-file-host"),
    )
}

fn error_widget(msg: &'static str) -> Box<dyn Widget> {
    Box::new(
        Padding::symmetric(10.0, 6.0)
            .child(Text::new(msg).class("audio-node-error")),
    )
}

pub fn spawn_load(
    path: PathBuf,
    buffer_sig: RwSignal<Option<Arc<AudioBuffer>>>,
    error_sig: RwSignal<Option<String>>,
) {
    let _ = thread::Builder::new()
        .name("synthos-audio-file-load".into())
        .spawn(move || match decode::decode_file(&path) {
            Ok(buf) => {
                buffer_sig.set(Some(Arc::new(buf)));
                error_sig.set(None);
            }
            Err(e) => {
                error_sig.set(Some(e));
                buffer_sig.set(None);
            }
        });
}
