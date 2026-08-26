//! Полноэкранный просмотр вложения.
//!
//! Живёт в overlay-слое через [`Portal`], монтируется один раз в
//! `pages::syn_chat::view()`. Открытие — клик по карточке вложения
//! (`super::attachments`), которая кладёт [`ViewerState`] в
//! `SynChatCtx.viewer`; закрытие — крестик, клик по фону или Esc (последние
//! два обрабатывает сам `Portal` в modal-режиме и зовёт `on_close`).
//!
//! ```text
//! ┌───────────────────────────────────────────────┐
//! │ photo.png  4032×3024 · 3,1 МБ    ⧉  ✕         │ ← шапка
//! ├───────────────────────────────────────────────┤
//! │  ‹        [ картинка / плеер / текст ]      ›  │ ← тело + листание
//! ├───────────────────────────────────────────────┤
//! │            −   100%   +   вписать      2 / 5  │ ← подвал
//! └───────────────────────────────────────────────┘
//! ```
//!
//! Тело зависит от модальности: картинка — в `PanZoomViewport` (колёсико
//! масштабирует, перетаскивание двигает), видео — ffmpeg-плеер syngui,
//! аудио — waveform с play/pause, документ — первые килобайты текста.

use std::sync::Arc;

use syngui::audio::AudioPlayer;
use syngui::core::sync::Mutex;
use syngui::mgui;
use syngui::prelude::*;
use syngui::video::{HwAccel, VideoPlayer};
use syngui::widgets::containers::PanZoomViewport;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::visual::video_player_view;
use syngui::widgets::visual::StaticWaveform;
use syngui::StyledWidget;

use crate::icons::{
    MI_CHEVRON_LEFT, MI_CHEVRON_RIGHT, MI_CLOSE, MI_FIT_SCREEN, MI_OPEN_IN_NEW, MI_PAUSE,
    MI_PLAY_ARROW, MI_ZOOM_IN, MI_ZOOM_OUT,
};
use crate::syn_chat::attach::{self, blobs};
use crate::syn_chat::state::{AttachmentKind, MsgAttachment, SynChatCtx};

/// Сколько символов документа показывать в просмотрщике. Полный текст
/// всё равно уходит в модель — здесь нужен именно быстрый взгляд.
const DOC_PREVIEW_CHARS: usize = 20_000;

/// Пределы масштабирования картинки колёсиком.
const ZOOM_MIN: f32 = 0.1;
const ZOOM_MAX: f32 = 12.0;

/// Общее состояние просмотрщика, живущее между перестроениями тела.
#[derive(Clone, Copy)]
struct ViewerSignals {
    zoom: RwSignal<f32>,
    pan: RwSignal<Point>,
    /// Декод и воспроизведение — общие с инлайн-карточкой ленты.
    audio: super::media_audio::AudioSignals,
}

pub fn view() -> impl Widget {
    // Portal требует RwSignal<bool>; держим отдельный и синхронизируем с
    // `ctx.viewer` эффектом — ровно как в `components::tool_confirm`.
    let is_open = use_signal(false);
    create_effect(move || {
        let ctx = use_context::<SynChatCtx>();
        let has = ctx.viewer.get().is_some();
        if is_open.get_untracked() != has {
            is_open.set(has);
        }
    });

    let signals = ViewerSignals {
        zoom: use_signal(1.0),
        pan: use_signal(Point::new(0.0, 0.0)),
        audio: super::media_audio::AudioSignals::new(),
    };
    // Плеер живёт вне реактивного дерева: его нужно останавливать при
    // закрытии и смене вложения, а не пересоздавать на каждый rebuild.
    let audio_player: Arc<Mutex<Option<AudioPlayer>>> = Arc::new(Mutex::new(None));

    let player_for_close = audio_player.clone();
    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .on_close(move || {
            super::media_audio::stop(&player_for_close);
            use_context::<SynChatCtx>().viewer.set(None);
        })
        .child(card(signals, audio_player))
}

fn card(
    signals: ViewerSignals,
    audio_player: Arc<Mutex<Option<AudioPlayer>>>,
) -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    move || {
        let ctx = use_context::<SynChatCtx>();
        let Some(state) = ctx.viewer.get() else {
            // Portal закрыт — содержимое всё равно не видно, но плеер надо
            // отпустить, иначе аудио продолжит играть в фоне.
            super::media_audio::stop(&audio_player);
            return DecoratedBox::new().class("media-viewer-empty");
        };
        let Some(item) = state.current().cloned() else {
            return DecoratedBox::new().class("media-viewer-empty");
        };

        let total = state.items.len();
        let index = state.index;

        let stage = DecoratedBox::new()
            .class("media-viewer-stage")
            .child(Column::new().children(vec![body(&item, signals, audio_player.clone())]));
        let stage_row = Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![
                nav_button(-1, total),
                Box::new(stage) as Box<dyn Widget>,
                nav_button(1, total),
            ]);

        DecoratedBox::new().class("media-viewer").child(mgui! {
            Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                header(&item),
                DecoratedBox::new().class("media-viewer-body") => [ stage_row ],
                footer(&item, signals, index, total),
            ]
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Шапка и подвал
// ─────────────────────────────────────────────────────────────────────────────

fn header(a: &MsgAttachment) -> impl Widget {
    let name = if a.original_name.is_empty() {
        a.kind.label().to_string()
    } else {
        a.original_name.clone()
    };
    let meta = attach::short_meta(a);
    let source = blobs::source_path(a);
    let for_save = a.clone();

    mgui! {
        DecoratedBox::new().class("media-viewer-header") => [
            Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(super::attachments::kind_icon(a.kind)).class("media-viewer-kind-icon"),
                Column::new().gap(1.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                    Text::new(name).class("media-viewer-title"),
                    Text::new(meta).class("media-viewer-subtitle"),
                ],
                DecoratedBox::new().class("grow"),
                ToolButton::new(crate::icons::MI_DOWNLOAD)
                    .tooltip(tr!("chat.media.save_as.tooltip"))
                    .on_click(move || super::media_inline::save_as(&for_save))
                    .class("media-viewer-action"),
                ToolButton::new(MI_OPEN_IN_NEW)
                    .tooltip(tr!("chat.media_viewer.open_external.tooltip"))
                    .on_click(move || open_externally(&source))
                    .class("media-viewer-action"),
                ToolButton::new(MI_CLOSE)
                    .tooltip(tr!("chat.media_viewer.close.tooltip"))
                    .on_click(close_viewer)
                    .class("media-viewer-action media-viewer-close"),
            ]
        ]
    }
}

fn footer(
    a: &MsgAttachment,
    signals: ViewerSignals,
    index: usize,
    total: usize,
) -> impl Widget {
    let zoomable = is_zoomable(a);
    let mut left: Vec<Box<dyn Widget>> = Vec::new();
    if zoomable {
        left.push(Box::new(
            ToolButton::new(MI_ZOOM_OUT)
                .tooltip(tr!("chat.media_viewer.zoom_out.tooltip"))
                .on_click(move || scale_by(signals, 1.0 / 1.25))
                .class("media-viewer-action"),
        ));
        left.push(Box::new(zoom_label(signals)));
        left.push(Box::new(
            ToolButton::new(MI_ZOOM_IN)
                .tooltip(tr!("chat.media_viewer.zoom_in.tooltip"))
                .on_click(move || scale_by(signals, 1.25))
                .class("media-viewer-action"),
        ));
        left.push(Box::new(
            ToolButton::new(MI_FIT_SCREEN)
                .tooltip(tr!("chat.media_viewer.fit.tooltip"))
                .on_click(move || reset_zoom(signals))
                .class("media-viewer-action"),
        ));
    }

    let counter = if total > 1 {
        format!("{} / {}", index + 1, total)
    } else {
        String::new()
    };

    mgui! {
        DecoratedBox::new().class("media-viewer-footer") => [
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).children(left),
                DecoratedBox::new().class("grow"),
                Text::new(counter).class("media-viewer-counter"),
            ]
        ]
    }
}

fn zoom_label(signals: ViewerSignals) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let z = signals.zoom.get();
        vec![Box::new(
            Text::new(format!("{:.0}%", z * 100.0)).class("media-viewer-zoom"),
        )]
    })
}

/// Стрелка листания. При одном вложении превращается в невидимую распорку,
/// чтобы картинка не «прыгала» между состояниями.
fn nav_button(delta: isize, total: usize) -> Box<dyn Widget> {
    if total < 2 {
        return Box::new(DecoratedBox::new().class("media-viewer-nav-spacer"));
    }
    let icon = if delta < 0 {
        MI_CHEVRON_LEFT
    } else {
        MI_CHEVRON_RIGHT
    };
    Box::new(
        ToolButton::new(icon)
            .tooltip(if delta < 0 { tr!("chat.media_viewer.prev.tooltip") } else { tr!("chat.media_viewer.next.tooltip") })
            .on_click(move || step(delta))
            .class("media-viewer-nav"),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Тело просмотрщика
// ─────────────────────────────────────────────────────────────────────────────

fn body(
    a: &MsgAttachment,
    signals: ViewerSignals,
    audio_player: Arc<Mutex<Option<AudioPlayer>>>,
) -> Box<dyn Widget> {
    match a.kind {
        _ if is_zoomable(a) => Box::new(image_stage(a, signals)),
        AttachmentKind::Video => video_stage(a),
        AttachmentKind::Audio => audio_stage(a, signals, audio_player),
        AttachmentKind::Document => Box::new(document_stage(a)),
        _ => Box::new(unsupported_stage(a)),
    }
}

/// Картинка можно двигать и масштабировать; SVG сюда же — syngui его
/// растеризует.
fn is_zoomable(a: &MsgAttachment) -> bool {
    matches!(a.kind, AttachmentKind::Image) || blobs::is_svg(a)
}

fn image_stage(a: &MsgAttachment, signals: ViewerSignals) -> impl Widget {
    let path = blobs::display_path(a);
    let image = Image::new(path.display().to_string())
        .fit(ImageFit::Contain)
        .class("media-viewer-image");

    PanZoomViewport::new()
        .zoom(signals.zoom)
        .pan(signals.pan)
        .zoom_range(ZOOM_MIN, ZOOM_MAX)
        .grid(false)
        .child(image)
        .class("media-viewer-panzoom")
}

fn video_stage(a: &MsgAttachment) -> Box<dyn Widget> {
    let path = blobs::source_path(a);
    match VideoPlayer::open_with_hwaccel(&path.display().to_string(), HwAccel::platform_default()) {
        Ok(player) => Box::new(
            DecoratedBox::new()
                .class("media-viewer-video")
                .child(video_player_view(Arc::new(Mutex::new(player)))),
        ),
        Err(e) => Box::new(error_stage(tr!("chat.media_viewer.video_open_error", error = e))),
    }
}

fn audio_stage(
    a: &MsgAttachment,
    signals: ViewerSignals,
    audio_player: Arc<Mutex<Option<AudioPlayer>>>,
) -> Box<dyn Widget> {
    super::media_audio::ensure_decoded(a, signals.audio, &audio_player);

    let name = a.original_name.clone();
    let duration = attach::format_duration(a.duration_ms);
    let player = audio_player.clone();

    let controls = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let buf = signals.audio.buf.get();
        let playing = signals.audio.playing.get();
        let pos = signals.audio.pos.get();
        let Some(buf) = buf else {
            return vec![Box::new(
                Text::new(tr!("chat.media.audio_decoding")).class("media-viewer-hint"),
            )];
        };
        let progress = {
            let total = buf.pcm.len() as f32 / (buf.sample_rate.max(1) as f32)
                / buf.channels.max(1) as f32;
            if total > 0.0 {
                (pos / total).clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let player = player.clone();
        vec![Box::new(mgui! {
            Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                StaticWaveform::new()
                    .pcm(Some(buf.clone()))
                    .progress(progress)
                    .height(140.0)
                    .class("media-viewer-waveform"),
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).main_axis_alignment(MainAxisAlignment::Center) => [
                    ToolButton::new(if playing { MI_PAUSE } else { MI_PLAY_ARROW })
                        .tooltip(if playing { tr!("chat.media.pause.tooltip") } else { tr!("chat.media.play.tooltip") })
                        .on_click(move || super::media_audio::toggle(&player, signals.audio))
                        .class("media-viewer-play"),
                ],
            ]
        }) as Box<dyn Widget>]
    });

    // Вертикальное центрирование делает сам Column (MainAxisAlignment::Center)
    // в блоке фиксированной высоты: `Center` тут не годится — он отдаёт
    // ребёнку loose-констрейнты, и колонка схлопывается в ноль.
    Box::new(mgui! {
        DecoratedBox::new().class("media-viewer-audio") => [
            Column::new()
                .gap(12.0)
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(name).class("media-viewer-audio-name"),
                    Text::new(duration).class("media-viewer-audio-meta"),
                    controls,
                ]
        ]
    })
}

fn document_stage(a: &MsgAttachment) -> impl Widget {
    let path = blobs::source_path(a);
    let text = match std::fs::read(&path) {
        Ok(bytes) => {
            let s = String::from_utf8_lossy(&bytes).into_owned();
            let mut out: String = s.chars().take(DOC_PREVIEW_CHARS).collect();
            if s.chars().count() > DOC_PREVIEW_CHARS {
                out.push_str("\n\n");
                out.push_str(&tr!("chat.media_viewer.doc_truncated"));
            }
            out
        }
        Err(e) => tr!("chat.media_viewer.doc_read_error", error = e),
    };
    DecoratedBox::new().class("media-viewer-doc").child(
        ScrollView::new()
            .vertical()
            .class("media-viewer-doc-scroll")
            .child(Text::new(text).class("media-viewer-doc-text")),
    )
}

fn unsupported_stage(a: &MsgAttachment) -> impl Widget {
    let icon = super::attachments::kind_icon(a.kind);
    let name = a.original_name.clone();
    let meta = attach::short_meta(a);
    mgui! {
        Center::new() => [
            Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(icon).class("media-viewer-big-icon"),
                Text::new(name).class("media-viewer-title"),
                Text::new(meta).class("media-viewer-subtitle"),
                Text::new(tr!("chat.media_viewer.unsupported"))
                    .class("media-viewer-hint"),
            ]
        ]
    }
}

fn error_stage(msg: String) -> impl Widget {
    mgui! {
        Center::new() => [
            Text::new(msg).class("media-viewer-hint"),
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Действия
// ─────────────────────────────────────────────────────────────────────────────

fn close_viewer() {
    use_context::<SynChatCtx>().viewer.set(None);
}

fn step(delta: isize) {
    let ctx = use_context::<SynChatCtx>();
    ctx.viewer.update(|v| {
        if let Some(state) = v.as_mut() {
            state.step(delta);
        }
    });
}

fn scale_by(signals: ViewerSignals, factor: f32) {
    let next = (signals.zoom.get_untracked() * factor).clamp(ZOOM_MIN, ZOOM_MAX);
    signals.zoom.set(next);
}

fn reset_zoom(signals: ViewerSignals) {
    signals.zoom.set(1.0);
    signals.pan.set(Point::new(0.0, 0.0));
}

fn open_externally(path: &std::path::Path) {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(cmd).arg(path).spawn() {
        log::warn!("[media-viewer] не удалось открыть {}: {e}", path.display());
    }
}
