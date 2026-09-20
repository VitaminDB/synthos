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
//! Тело зависит от модальности: картинка — сцена [`super::image_stage`]
//! (размытый фон из неё же, масштаб к курсору, полосы прокрутки, поворот,
//! лента миниатюр и панель инструментов поверх; подвала у неё нет), видео — плеер
//! [`crate::components::video_player`] во всю сцену (умеет разворачиваться
//! на всё окно), аудио — waveform с play/pause, документ — первые килобайты
//! текста. Подвал есть только когда в нём что-то есть — счётчик вложений.
//!
//! Окно картинки и видео можно развернуть на всё окно приложения (кнопка в
//! шапке, двойной щелчок по шапке, `M`) и тянуть за края. Карточка стоит по
//! центру, поэтому край уходит на столько же и с другой стороны.

use std::sync::Arc;

use syngui::audio::AudioPlayer;
use syngui::core::sync::Mutex;
use syngui::mgui;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::video::{HwAccel, VideoPlayer};
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::visual::StaticWaveform;
use syngui::StyledWidget;

use crate::components::drag_handle::{DragHandle, DragPhase};
use crate::components::video_player::{FullscreenCtl, VideoPlayerView};
use crate::icons::{
    MI_CHEVRON_LEFT, MI_CHEVRON_RIGHT, MI_CLOSE, MI_CLOSE_FULLSCREEN, MI_CONTENT_COPY,
    MI_OPEN_IN_FULL, MI_OPEN_IN_NEW, MI_PAUSE, MI_PLAY_ARROW,
};
use crate::syn_chat::attach::{self, blobs};
use crate::syn_chat::state::{AttachmentKind, MsgAttachment, SynChatCtx};

/// Сколько символов документа показывать в просмотрщике. Полный текст
/// всё равно уходит в модель — здесь нужен именно быстрый взгляд.
const DOC_PREVIEW_CHARS: usize = 20_000;

/// Размер карточки, пока её не тянули (он же в `.media-viewer`), наименьший
/// при ресайзе и доля окна приложения, больше которой карточка не бывает.
const CARD_DEFAULT: (f32, f32) = (1280.0, 860.0);
const CARD_MIN: (f32, f32) = (520.0, 400.0);
const CARD_MAX_FRACTION: (f32, f32) = (0.96, 0.94);

/// Общее состояние просмотрщика, живущее между перестроениями тела.
#[derive(Clone, Copy)]
struct ViewerSignals {
    image: super::image_stage::ImageSignals,
    win: WinSignals,
    /// Декод и воспроизведение — общие с инлайн-карточкой ленты.
    audio: super::media_audio::AudioSignals,
    full: FullSignals,
}

/// Окно просмотрщика: размер после ресайза и «развёрнуто».
#[derive(Clone, Copy)]
struct WinSignals {
    /// `None` — размер из MSS.
    size: RwSignal<Option<Size>>,
    maximized: RwSignal<bool>,
    /// Размер в момент, когда край взяли мышью. Живёт в сигнале, а не в
    /// замыкании ручки: каждое движение пересобирает карточку и с ней
    /// замыкание, а смещение мыши считается от точки нажатия.
    resize_base: RwSignal<Option<Size>>,
}

/// «Во весь экран»: карточка на всё окно, окно — полноэкранное.
#[derive(Clone, Copy)]
struct FullSignals {
    active: RwSignal<bool>,
    /// Окно развернули мы (а не F11 до нас) — нам его и возвращать.
    window_ours: RwSignal<bool>,
}

/// Открытый плеер текущего видео и sha его вложения. Живёт вне реактивного
/// дерева: переход во весь экран пересобирает карточку, а ролик должен
/// играть дальше с того же места, а не открываться заново.
type VideoSlot = Arc<Mutex<Option<(String, Arc<Mutex<VideoPlayer>>)>>>;

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
        image: super::image_stage::ImageSignals::new(),
        win: WinSignals {
            size: use_signal(None),
            maximized: use_signal(false),
            resize_base: use_signal(None),
        },
        audio: super::media_audio::AudioSignals::new(),
        full: FullSignals {
            active: use_signal(false),
            window_ours: use_signal(false),
        },
    };
    // Плееры живут вне реактивного дерева: их нужно останавливать при
    // закрытии и смене вложения, а не пересоздавать на каждый rebuild.
    let audio_player: Arc<Mutex<Option<AudioPlayer>>> = Arc::new(Mutex::new(None));
    let video: VideoSlot = Arc::new(Mutex::new(None));

    let player_for_close = audio_player.clone();
    let video_for_close = video.clone();
    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .on_close(move || {
            super::media_audio::stop(&player_for_close);
            release_video(&video_for_close);
            set_full(signals.full, false);
            use_context::<SynChatCtx>().viewer.set(None);
        })
        .child(card(signals, audio_player, video))
}

fn card(
    signals: ViewerSignals,
    audio_player: Arc<Mutex<Option<AudioPlayer>>>,
    video: VideoSlot,
) -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    move || {
        let ctx = use_context::<SynChatCtx>();
        let Some(state) = ctx.viewer.get() else {
            // Portal закрыт — содержимое всё равно не видно, но плееры надо
            // отпустить, иначе звук продолжит играть в фоне.
            super::media_audio::stop(&audio_player);
            release_video(&video);
            set_full(signals.full, false);
            return DecoratedBox::new().class("media-viewer-empty");
        };
        let Some(item) = state.current().cloned() else {
            return DecoratedBox::new().class("media-viewer-empty");
        };

        let total = state.items.len();
        let index = state.index;
        let is_image = is_zoomable(&item);
        let is_video = matches!(item.kind, AttachmentKind::Video) && !is_image;
        if !is_video {
            release_video(&video);
        }
        // Развернуть и тянуть можно то, что умеет занять любое место:
        // у аудио и документа сцена фиксированного размера.
        let sizable = is_image || is_video;
        let full = sizable && signals.full.active.get();

        if is_video && full {
            // Во весь экран — только кадр и его панель, без шапки и листания.
            return DecoratedBox::new()
                .class("media-viewer media-viewer-full")
                .child(video_stage(&item, signals.full, &video, true));
        }
        if is_image {
            let stage = super::image_stage::image_stage(
                &item,
                &state,
                signals.image,
                stage_host(&item, signals, full),
            );
            if full {
                return DecoratedBox::new()
                    .class("media-viewer media-viewer-full")
                    .child(stage);
            }
            let body = DecoratedBox::new().class("media-viewer-body").child(
                Row::new()
                    .gap(0.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(DecoratedBox::new().class("media-viewer-image-stage").child(stage)),
            );
            return window(signals.win, vec![Box::new(header(&item, signals.win, true)), Box::new(body)]);
        }

        let stage: Box<dyn Widget> = if is_video {
            Box::new(
                DecoratedBox::new()
                    .class("media-viewer-video-stage")
                    .child(video_stage(&item, signals.full, &video, false)),
            )
        } else {
            Box::new(
                DecoratedBox::new()
                    .class("media-viewer-stage")
                    .child(Column::new().children(vec![body(
                        &item,
                        signals,
                        audio_player.clone(),
                    )])),
            )
        };
        // Одиночному видео распорки вместо стрелок не нужны: кадр — от края
        // до края карточки.
        let row_items = if is_video && total < 2 {
            vec![stage]
        } else {
            vec![nav_button(-1, total), stage, nav_button(1, total)]
        };
        let stage_row = Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(row_items);

        let mut rows: Vec<Box<dyn Widget>> = vec![
            Box::new(header(&item, signals.win, sizable)),
            Box::new(DecoratedBox::new().class("media-viewer-body").child(stage_row)),
        ];
        // Пустой подвал у видео и аудио оставлял под сценой голую полосу.
        if total > 1 {
            rows.push(Box::new(footer(index, total)));
        }
        if sizable {
            return window(signals.win, rows);
        }
        DecoratedBox::new().class("media-viewer").child(
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(rows),
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Окно: размер, разворот, ручки по краям
// ─────────────────────────────────────────────────────────────────────────────

/// Карточка изменяемого размера: содержимое и поверх него ручки по краям.
fn window(win: WinSignals, rows: Vec<Box<dyn Widget>>) -> StyledWidget<DecoratedBox> {
    let maximized = win.maximized.get();
    let size = win.size.get();

    let content = Column::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(rows);
    let mut layers = Stack::new().fit(StackFit::Expand).child(content);
    if !maximized {
        layers = layers
            .child(edge_layer(win, Edge::Left))
            .child(edge_layer(win, Edge::Right))
            .child(edge_layer(win, Edge::Bottom))
            .child(edge_layer(win, Edge::BottomLeft))
            .child(edge_layer(win, Edge::BottomRight));
    }

    let card = DecoratedBox::new().class(if maximized {
        "media-viewer media-viewer-max"
    } else {
        "media-viewer"
    });
    let card = match (maximized, size) {
        (false, Some(s)) => card
            .style("width", StyleValue::px(s.width))
            .style("height", StyleValue::px(s.height)),
        _ => card,
    };
    card.child(layers)
}

#[derive(Clone, Copy)]
enum Edge {
    Left,
    Right,
    Bottom,
    BottomLeft,
    BottomRight,
}

impl Edge {
    /// Во сколько раз смещение мыши меняет ширину и высоту. Карточка стоит
    /// по центру: чтобы край шёл за курсором, размер растёт вдвое быстрее.
    fn factors(self) -> (f32, f32) {
        match self {
            Edge::Left => (-2.0, 0.0),
            Edge::Right => (2.0, 0.0),
            Edge::Bottom => (0.0, 2.0),
            Edge::BottomLeft => (-2.0, 2.0),
            Edge::BottomRight => (2.0, 2.0),
        }
    }

    fn class(self) -> &'static str {
        match self {
            Edge::Left | Edge::Right => "media-viewer-grip media-viewer-grip-x",
            Edge::Bottom => "media-viewer-grip media-viewer-grip-y",
            Edge::BottomLeft => "media-viewer-grip media-viewer-grip-sw",
            Edge::BottomRight => "media-viewer-grip media-viewer-grip-se",
        }
    }
}

/// Слой во всю карточку с ручкой у нужного края. Пустое место слоя событий
/// не ловит — они уходят содержимому под ним.
fn edge_layer(win: WinSignals, edge: Edge) -> Box<dyn Widget> {
    let (fx, fy) = edge.factors();
    let grip = DecoratedBox::new().class(edge.class()).child(
        DragHandle::new(DecoratedBox::new().class("media-viewer-grip-fill")).on_drag(
            move |phase| match phase {
                DragPhase::Move { delta, .. } => {
                    let base = win.resize_base.get_untracked().unwrap_or_else(|| {
                        let now = card_size(win.size.get_untracked(), window_size());
                        win.resize_base.set(Some(now));
                        now
                    });
                    let next = Size::new(base.width + delta.x * fx, base.height + delta.y * fy);
                    win.size.set(Some(clamp_card(next, window_size())));
                }
                DragPhase::End => win.resize_base.set(None),
            },
        ),
    );
    match edge {
        Edge::Left | Edge::Right => Box::new(
            Row::new()
                .main_axis_alignment(if matches!(edge, Edge::Left) {
                    MainAxisAlignment::Start
                } else {
                    MainAxisAlignment::End
                })
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(grip),
        ),
        Edge::Bottom => Box::new(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::End)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(grip),
        ),
        Edge::BottomLeft | Edge::BottomRight => Box::new(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::End)
                .cross_axis_alignment(if matches!(edge, Edge::BottomLeft) {
                    CrossAxisAlignment::Start
                } else {
                    CrossAxisAlignment::End
                })
                .child(grip),
        ),
    }
}

/// Размер окна приложения в логических пикселях. `None` — окна нет (тесты).
fn window_size() -> Option<Size> {
    let w = syngui::signal::primary_window()?;
    let w = w.winit_window();
    let px = w.inner_size();
    let k = w.scale_factor() as f32;
    (px.width > 0 && px.height > 0 && k > 0.0)
        .then(|| Size::new(px.width as f32 / k, px.height as f32 / k))
}

/// Пределы карточки: не меньше [`CARD_MIN`], не больше доли окна.
fn clamp_card(size: Size, window: Option<Size>) -> Size {
    let (max_w, max_h) = match window {
        Some(w) => (w.width * CARD_MAX_FRACTION.0, w.height * CARD_MAX_FRACTION.1),
        None => (f32::INFINITY, f32::INFINITY),
    };
    Size::new(
        size.width.min(max_w).max(CARD_MIN.0.min(max_w)),
        size.height.min(max_h).max(CARD_MIN.1.min(max_h)),
    )
}

/// Сколько карточка занимает сейчас: заданный размер или размер из MSS — в
/// пределах окна, как их ограничит раскладка.
fn card_size(size: Option<Size>, window: Option<Size>) -> Size {
    clamp_card(
        size.unwrap_or(Size::new(CARD_DEFAULT.0, CARD_DEFAULT.1)),
        window,
    )
}

fn toggle_maximize(win: WinSignals) {
    win.maximized.update(|m| *m = !*m);
}

fn stage_host(
    item: &MsgAttachment,
    signals: ViewerSignals,
    fullscreen: bool,
) -> super::image_stage::StageHost {
    let for_copy = item.clone();
    super::image_stage::StageHost {
        fullscreen,
        toggle_fullscreen: Arc::new(move || set_full(signals.full, !signals.full.active.get_untracked())),
        toggle_maximize: Arc::new(move || toggle_maximize(signals.win)),
        copy: Arc::new(move || copy_image(&for_copy)),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Шапка и подвал
// ─────────────────────────────────────────────────────────────────────────────

fn header(a: &MsgAttachment, win: WinSignals, sizable: bool) -> impl Widget {
    let name = if a.original_name.is_empty() {
        a.kind.label().to_string()
    } else {
        a.original_name.clone()
    };
    let meta = attach::short_meta(a);
    let source = blobs::source_path(a);
    let for_save = a.clone();

    let mut actions: Vec<Box<dyn Widget>> = Vec::new();
    if is_zoomable(a) && !blobs::is_svg(a) {
        let for_copy = a.clone();
        actions.push(Box::new(
            ToolButton::new(MI_CONTENT_COPY)
                .tooltip(tr!("chat.media_viewer.copy.tooltip"))
                .on_click(move || copy_image(&for_copy))
                .class("media-viewer-action"),
        ));
    }
    actions.push(Box::new(
        ToolButton::new(crate::icons::MI_DOWNLOAD)
            .tooltip(tr!("chat.media.save_as.tooltip"))
            .on_click(move || super::media_inline::save_as(&for_save))
            .class("media-viewer-action"),
    ));
    actions.push(Box::new(
        ToolButton::new(MI_OPEN_IN_NEW)
            .tooltip(tr!("chat.media_viewer.open_external.tooltip"))
            .on_click(move || open_externally(&source))
            .class("media-viewer-action"),
    ));
    if sizable {
        let maximized = win.maximized.get();
        actions.push(Box::new(DecoratedBox::new().class("media-viewer-action-sep")));
        actions.push(Box::new(
            ToolButton::new(if maximized { MI_CLOSE_FULLSCREEN } else { MI_OPEN_IN_FULL })
                .tooltip(if maximized {
                    tr!("chat.media_viewer.restore.tooltip")
                } else {
                    tr!("chat.media_viewer.maximize.tooltip")
                })
                .on_click(move || toggle_maximize(win))
                .class("media-viewer-action"),
        ));
    }
    actions.push(Box::new(
        ToolButton::new(MI_CLOSE)
            .tooltip(tr!("chat.media_viewer.close.tooltip"))
            .on_click(close_viewer)
            .class("media-viewer-action media-viewer-close"),
    ));

    let title = mgui! {
        Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(super::attachments::kind_icon(a.kind)).class("media-viewer-kind-icon"),
            Column::new().gap(1.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                Text::new(name).class("media-viewer-title"),
                Text::new(meta).class("media-viewer-subtitle"),
            ],
        ]
    };
    // Двойной щелчок по названию разворачивает окно — как у заголовка
    // обычного окна. Кнопки справа в жест не входят.
    let title: Box<dyn Widget> = if sizable {
        Box::new(
            // `grow` — самой обёртке: строка шапки растягивает своих прямых
            // детей, до класса вложенного бокса ей дела нет.
            GestureDetector::new()
                .on_double_click(move || toggle_maximize(win))
                .class("grow")
                .child(DecoratedBox::new().class("media-viewer-title-area").child(title)),
        )
    } else {
        Box::new(DecoratedBox::new().class("media-viewer-title-area").child(title))
    };

    DecoratedBox::new().class("media-viewer-header").child(
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(title)
            .child(
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(actions),
            ),
    )
}

/// Подвал сцен без своей панели (видео, аудио, документ): счётчик вложений.
fn footer(index: usize, total: usize) -> impl Widget {
    mgui! {
        DecoratedBox::new().class("media-viewer-footer") => [
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().class("grow"),
                Text::new(format!("{} / {}", index + 1, total)).class("media-viewer-counter"),
            ]
        ]
    }
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

fn video_stage(
    a: &MsgAttachment,
    full: FullSignals,
    slot: &VideoSlot,
    active: bool,
) -> Box<dyn Widget> {
    match open_video(a, slot) {
        Ok(player) => {
            let fullscreen = FullscreenCtl {
                active,
                toggle: Arc::new(move || set_full(full, !full.active.get_untracked())),
            };
            Box::new(VideoPlayerView::file(player).fullscreen(fullscreen).build())
        }
        Err(e) => Box::new(error_stage(tr!("chat.media_viewer.video_open_error", error = e))),
    }
}

/// С какого места открыть видео: карточка ленты кладёт сюда позицию, где
/// её плеер встал на паузу перед «во весь экран», — ролик продолжается, а не
/// начинается заново.
static START_AT: std::sync::Mutex<Option<(String, f64)>> = std::sync::Mutex::new(None);

pub fn start_video_at(sha: &str, t: f64) {
    if let Ok(mut g) = START_AT.lock() {
        *g = Some((sha.to_string(), t));
    }
}

/// Плеер вложения: уже открытый, если это то же видео (карточку
/// пересобрали), иначе новый — прежний закрывается.
fn open_video(
    a: &MsgAttachment,
    slot: &VideoSlot,
) -> std::result::Result<Arc<Mutex<VideoPlayer>>, String> {
    if let Ok(guard) = slot.lock() {
        if let Some((sha, p)) = guard.as_ref() {
            if *sha == a.sha256 {
                return Ok(p.clone());
            }
        }
    }
    release_video(slot);
    let path = blobs::source_path(a);
    let mut player =
        VideoPlayer::open_with_hwaccel(&path.display().to_string(), HwAccel::platform_default())
            .map_err(|e| e.to_string())?;
    let start = START_AT.lock().ok().and_then(|mut g| g.take());
    if let Some((_, t)) = start.filter(|(sha, t)| *sha == a.sha256 && *t > 0.5) {
        if let Err(e) = player.seek(t) {
            log::warn!("[media-viewer] продолжить с {t:.1}s: {e}");
        }
    }
    let player = Arc::new(Mutex::new(player));
    if let Ok(mut guard) = slot.lock() {
        *guard = Some((a.sha256.clone(), player.clone()));
    }
    Ok(player)
}

/// Закрыть видео просмотрщика: звук — сразу, декодер — в фоне (его потоки
/// останавливаются с `join`, кадр UI ждать этого не должен).
fn release_video(slot: &VideoSlot) {
    let old = slot.lock().ok().and_then(|mut s| s.take());
    if let Some((_, player)) = old {
        if let Ok(mut p) = player.lock() {
            p.pause();
        }
        std::thread::spawn(move || drop(player));
    }
}

/// Карточка на всё окно и окно в полноэкранный режим. Выход возвращает окно
/// как было: если его развернули F11 ещё до нас, оно таким и останется.
fn set_full(full: FullSignals, on: bool) {
    if full.active.get_untracked() == on {
        return;
    }
    full.active.set(on);
    let window_full = window_is_fullscreen();
    if on {
        if !window_full {
            syngui::signal::toggle_fullscreen();
            full.window_ours.set(true);
        }
    } else if full.window_ours.get_untracked() {
        full.window_ours.set(false);
        if window_full {
            syngui::signal::toggle_fullscreen();
        }
    }
}

fn window_is_fullscreen() -> bool {
    syngui::signal::primary_window().is_some_and(|w| w.winit_window().fullscreen().is_some())
}

fn audio_stage(
    a: &MsgAttachment,
    signals: ViewerSignals,
    audio_player: Arc<Mutex<Option<AudioPlayer>>>,
) -> Box<dyn Widget> {
    super::media_audio::ensure_decoded(a, signals.audio, &audio_player);

    let name = a.original_name.clone();
    let duration = attach::format_duration(a.duration_ms);

    // Волна и кнопка — в разных Reactive: волна перерисовывается на каждый
    // тик позиции (12 раз в секунду), и общий блок пересобирал бы вместе с
    // ней кнопку, теряя клик между press и release — пауза не срабатывала.
    let wave_player = audio_player.clone();
    let wave = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(buf) = signals.audio.buf.get() else {
            return vec![Box::new(
                Text::new(tr!("chat.media.audio_decoding")).class("media-viewer-hint"),
            )];
        };
        let seek_player = wave_player.clone();
        vec![Box::new(
            StaticWaveform::new()
                .pcm(Some(buf))
                .progress(signals.audio.progress())
                .on_seek(move |t| super::media_audio::seek(&seek_player, signals.audio, t))
                .height(140.0)
                .class("media-viewer-waveform"),
        )]
    });

    let btn_player = audio_player.clone();
    let play_btn = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let playing = signals.audio.playing.get();
        let btn_player = btn_player.clone();
        vec![Box::new(
            ToolButton::new(if playing { MI_PAUSE } else { MI_PLAY_ARROW })
                .tooltip(if playing {
                    tr!("chat.media.pause.tooltip")
                } else {
                    tr!("chat.media.play.tooltip")
                })
                .on_click(move || super::media_audio::toggle(&btn_player, signals.audio))
                .class("media-viewer-play"),
        )]
    });

    let controls = mgui! {
        Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            wave,
            Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).main_axis_alignment(MainAxisAlignment::Center) => [
                play_btn,
            ],
        ]
    };

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
                    Reactive::new(move || -> Vec<Box<dyn Widget>> {
                        let pos = signals.audio.pos.get();
                        let label = match pos > 0.0 {
                            true => format!(
                                "{} / {duration}",
                                attach::format_duration((pos * 1000.0) as u64)
                            ),
                            false => duration.clone(),
                        };
                        vec![Box::new(Text::new(label).class("media-viewer-audio-meta"))]
                    }),
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

/// Картинку — в буфер: PNG (вставится в редактор или мессенджер) и ссылка на
/// файл (вставится в файловый менеджер), текстом — путь.
fn copy_image(a: &MsgAttachment) {
    let path = blobs::display_path(a);
    let png = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
        std::fs::read(&path).map_err(|e| e.to_string())
    } else {
        image::open(&path).map_err(|e| e.to_string()).and_then(|img| {
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png)
                .map(|()| out.into_inner())
                .map_err(|e| e.to_string())
        })
    };
    let uris = syngui::clipboard::uri_list(&[&path]);
    let mut formats: Vec<(&str, &[u8])> = vec![("text/uri-list", uris.as_bytes())];
    match &png {
        Ok(bytes) => formats.insert(0, ("image/png", bytes.as_slice())),
        Err(e) => log::warn!("[media-viewer] копия {} как PNG: {e}", path.display()),
    }
    syngui::clipboard::copy_rich(&path.display().to_string(), &formats);
    use_context::<crate::context::AppCtx>()
        .notifications
        .success(tr!("chat.media_viewer.copied"));
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
