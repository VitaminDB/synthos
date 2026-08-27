//! Инлайн-медиа в пузыре чата: результат прогона играет прямо в ленте.
//!
//! Пайплайн отдаёт видео/аудио/картинки вложениями (см.
//! `syn_chat::pipeline_run::collect_artifacts` и `collect_viewer_outputs`), и
//! смотреть их через модальный просмотрщик — лишний клик: результат нужен
//! там же, где о нём написано. Карточка показывает превью, по кнопке
//! разворачивает настоящий плеер и даёт «Сохранить» — забрать файл из CAS
//! в удобное место.
//!
//! ```text
//! ┌──────────────────────────────┐
//! │        превью / плеер        │  ← видео: постер + ⏵, потом VideoPlayer
//! │                              │     аудио: волна + ⏵/⏸
//! ├──────────────────────────────┤
//! │ имя · 9 МБ · 768×448   ⤓  ↗  │  ← Сохранить · Открыть внешне
//! └──────────────────────────────┘
//! ```

use std::sync::Arc;

use syngui::core::sync::Mutex;
use syngui::mgui;
use syngui::prelude::*;
use syngui::video::{HwAccel, VideoPlayer};
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::visual::{video_player_view, StaticWaveform};

use crate::context::AppCtx;
use crate::icons::{MI_DOWNLOAD, MI_FIT_SCREEN, MI_OPEN_IN_NEW, MI_PAUSE, MI_PLAY_ARROW};
use crate::syn_chat::attach::{self, blobs};
use crate::syn_chat::state::{AttachmentKind, MsgAttachment};

use super::media_audio::{self, AudioSignals, PlayerSlot};

/// Показывать ли вложение инлайн-плеером (иначе — обычная карточка-плитка).
pub fn is_inline(a: &MsgAttachment) -> bool {
    matches!(
        a.kind,
        AttachmentKind::Image | AttachmentKind::Video | AttachmentKind::Audio
    )
}

/// Карточка с плеером. `on_open` — клик по превью (полноэкранный просмотр).
pub fn media_card<F>(a: &MsgAttachment, on_open: F) -> Box<dyn Widget>
where
    F: Fn() + Send + Sync + 'static,
{
    let on_open = Arc::new(on_open);
    let stage: Box<dyn Widget> = match a.kind {
        AttachmentKind::Video => video_stage(a),
        AttachmentKind::Audio => audio_stage(a),
        _ => image_stage(a, {
            let on_open = on_open.clone();
            move || on_open()
        }),
    };
    let actions = actions_row(a, on_open);
    Box::new(
        DecoratedBox::new().class("chat-media-card").child(
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![stage, actions]),
        ),
    )
}

/// Нижняя строка: подпись и действия над файлом.
fn actions_row(a: &MsgAttachment, on_open: Arc<dyn Fn() + Send + Sync>) -> Box<dyn Widget> {
    let name = if a.original_name.is_empty() {
        a.kind.label().to_string()
    } else {
        a.original_name.clone()
    };
    let meta = attach::short_meta(a);
    let save = a.clone();
    let open = a.clone();
    Box::new(
        DecoratedBox::new().class("chat-media-actions").child(mgui! {
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                Column::new().gap(1.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                    Text::new(name).class("chat-media-name"),
                    Text::new(meta).class("chat-media-meta"),
                ],
                Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    ToolButton::new(MI_FIT_SCREEN)
                        .tooltip(tr!("chat.media.fullscreen.tooltip"))
                        .on_click(move || on_open())
                        .class("chat-media-btn"),
                    ToolButton::new(MI_DOWNLOAD)
                        .tooltip(tr!("chat.media.save_as.tooltip"))
                        .on_click(move || save_as(&save))
                        .class("chat-media-btn"),
                    ToolButton::new(MI_OPEN_IN_NEW)
                        .tooltip(tr!("chat.media.open_external.tooltip"))
                        .on_click(move || open_externally(&open))
                        .class("chat-media-btn"),
                ],
            ]
        }),
    )
}

/// Видео: постер с кнопкой ⏵, по нажатию — настоящий плеер на том же месте.
/// Ленивость намеренная: декодер на каждое видео в ленте съел бы память и
/// GPU, а большинство роликов пользователь не переоткрывает.
fn video_stage(a: &MsgAttachment) -> Box<dyn Widget> {
    let started = use_signal(false);
    let path = blobs::source_path(a).display().to_string();
    let poster = blobs::preview_path(a).map(|p| p.display().to_string());
    let duration = (a.duration_ms > 0).then(|| attach::format_duration(a.duration_ms));
    // Плеер создаём один раз и держим вне реактивного дерева.
    let player: Arc<Mutex<Option<Arc<Mutex<VideoPlayer>>>>> = Arc::new(Mutex::new(None));

    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if !started.get() {
            let poster: Box<dyn Widget> = match &poster {
                Some(p) => Box::new(
                    Image::new(p.clone())
                        .fit(ImageFit::Cover)
                        .class("chat-media-poster"),
                ),
                None => Box::new(DecoratedBox::new().class("chat-media-poster-empty")),
            };
            let play = mgui! {
                Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    ToolButton::new(MI_PLAY_ARROW)
                        .tooltip(tr!("chat.media.play.tooltip"))
                        .on_click(move || started.set(true))
                        .class("chat-media-play"),
                ]
            };
            let mut layers: Vec<Box<dyn Widget>> = vec![poster, Box::new(play)];
            if let Some(d) = &duration {
                layers.push(Box::new(badge(d.clone())));
            }
            // Клик по кадру = запустить: именно этого ждут от постера с ⏵.
            // Полноэкранный просмотр — отдельной кнопкой в строке действий.
            return vec![Box::new(
                GestureDetector::new()
                    .cursor(syngui::input::CursorIcon::Pointer)
                    .on_click(move || started.set(true))
                    .child(
                        DecoratedBox::new()
                            .class("chat-media-stage")
                            .child(Stack::new().fit(StackFit::Expand).children(layers)),
                    ),
            ) as Box<dyn Widget>];
        }

        let mut guard = match player.lock() {
            Ok(g) => g,
            Err(_) => return vec![],
        };
        if guard.is_none() {
            // Программный декодер намеренно: NVDEC держит контекст в той же
            // VRAM, куда возвращается чат-LLM после прогона, и превью ролика
            // в ленте не стоит отнятых у модели мегабайт.
            match VideoPlayer::open_with_hwaccel(&path, HwAccel::None) {
                Ok(p) => *guard = Some(Arc::new(Mutex::new(p))),
                Err(e) => {
                    return vec![Box::new(
                        DecoratedBox::new()
                            .class("chat-media-stage")
                            .child(Center::new().child(
                                Text::new(tr!("chat.media.video_open_error", error = e)).class("chat-media-meta"),
                            )),
                    )]
                }
            }
        }
        let p = guard.as_ref().expect("плеер создан").clone();
        vec![Box::new(
            DecoratedBox::new()
                .class("chat-media-stage")
                .child(video_player_view(p)),
        )]
    }))
}

/// Аудио: волна с перемоткой и ⏵/⏸. Декод запускается при первом показе.
///
/// Три отдельных `Reactive` — не косметика: позиция тикает 12 раз в секунду,
/// и общий на всю карточку пересобирал бы вместе с волной и кнопку. Клик по
/// ней терялся между press и release (виджет успевал смениться), из-за чего
/// пауза не срабатывала и трек доигрывал до конца.
fn audio_stage(a: &MsgAttachment) -> Box<dyn Widget> {
    let signals = AudioSignals::new();
    let player: PlayerSlot = media_audio::new_slot();
    media_audio::ensure_decoded(a, signals, &player);
    let duration = attach::format_duration(a.duration_ms);

    let wave_player = player.clone();
    let wave = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(b) = signals.buf.get() else {
            return vec![Box::new(Center::new().child(
                Text::new(tr!("chat.media.audio_decoding")).class("chat-media-meta"),
            ))];
        };
        let seek_player = wave_player.clone();
        vec![Box::new(
            StaticWaveform::new()
                .pcm(Some(b))
                .progress(signals.progress())
                .on_seek(move |t| media_audio::seek(&seek_player, signals, t))
                .height(64.0)
                .class("chat-media-waveform"),
        )]
    });

    let btn_player = player.clone();
    let play_btn = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let playing = signals.playing.get();
        let btn_player = btn_player.clone();
        vec![Box::new(
            ToolButton::new(if playing { MI_PAUSE } else { MI_PLAY_ARROW })
                .tooltip(if playing {
                    tr!("chat.media.pause.tooltip")
                } else {
                    tr!("chat.media.play.tooltip")
                })
                .on_click(move || media_audio::toggle(&btn_player, signals))
                .class("chat-media-play-small"),
        )]
    });

    let timecode = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let pos = signals.pos.get();
        let label = match pos > 0.0 {
            true => format!("{} / {duration}", attach::format_duration((pos * 1000.0) as u64)),
            false => duration.clone(),
        };
        vec![Box::new(Text::new(label).class("chat-media-meta"))]
    });

    let controls = mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
            play_btn,
            timecode,
        ]
    };
    Box::new(
        DecoratedBox::new().class("chat-media-audio").child(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![Box::new(wave) as Box<dyn Widget>, Box::new(controls)]),
        ),
    )
}

/// Картинка: превью во всю ширину карточки, клик — полноэкранный просмотр.
fn image_stage<F>(a: &MsgAttachment, on_open: F) -> Box<dyn Widget>
where
    F: Fn() + Send + Sync + 'static,
{
    let preview: Box<dyn Widget> = match blobs::preview_path(a) {
        Some(p) => Box::new(
            Image::new(p.display().to_string())
                .fit(ImageFit::Contain)
                .class("chat-media-image"),
        ),
        None => Box::new(DecoratedBox::new().class("chat-media-poster-empty")),
    };
    Box::new(
        GestureDetector::new()
            .cursor(syngui::input::CursorIcon::Pointer)
            .on_click(move || on_open())
            .child(
                DecoratedBox::new()
                    .class("chat-media-stage")
                    .child(Column::new().children(vec![preview])),
            ),
    )
}

fn badge(text: String) -> impl Widget {
    DecoratedBox::new()
        .class("attachment-card-badge-wrap")
        .child(mgui! {
            Column::new().main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Start) => [
                DecoratedBox::new().class("attachment-card-badge") => [
                    Text::new(text).class("attachment-card-badge-text"),
                ]
            ]
        })
}

/// «Сохранить как…»: копия blob'а из CAS в выбранный файл. Диалог — в
/// отдельном потоке: rfd блокирует, а UI-поток обязан оставаться живым.
pub fn save_as(a: &MsgAttachment) {
    let src = blobs::source_path(a);
    let name = if a.original_name.is_empty() {
        format!("{}.{}", a.kind.label(), a.ext)
    } else {
        a.original_name.clone()
    };
    std::thread::spawn(move || {
        let Some(dest) = rfd::FileDialog::new()
            .set_title(tr!("chat.media.save_dialog.title", name = name))
            .set_file_name(&name)
            .save_file()
        else {
            return;
        };
        let res = std::fs::copy(&src, &dest);
        syngui::async_runtime::run_on_main_thread(move || {
            let notif = use_context::<AppCtx>().notifications.clone();
            match res {
                Ok(_) => notif.success(tr!("chat.media.save.success", path = dest.display())),
                Err(e) => notif.error(tr!("chat.media.save.error", error = e)),
            }
        });
    });
}

fn open_externally(a: &MsgAttachment) {
    let path = blobs::source_path(a);
    std::thread::spawn(move || {
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
    });
}
