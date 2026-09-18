//! Видеоплеер просмотрщика вложений — по образцу плеера tv_rezka: кадр во
//! всю сцену, панель управления поверх него с автоскрытием.
//!
//! ```text
//! ┌──────────────────────────────────────────────────┐
//! │                                                  │
//! │                    ( ▶ )                         │ ← на паузе: большая ⏵
//! │                                                  │
//! │▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓│ ← градиент к низу
//! │━━━━━━━━━━━━━━━━━━●───────────────────────────────│ ← перемотка во всю ширину
//! │ 0:12 / 1:04        ⟲10  ( ⏸ )  10⟳      🔊━━━  ⛶ │
//! └──────────────────────────────────────────────────┘
//! ```
//!
//! Клик по кадру — пауза, двойной — во весь экран. Панель прячется через
//! [`HIDE_AFTER`] без движения мыши, пока идёт воспроизведение; на паузе
//! видна всегда. Клавиши: пробел/K — пауза, ←/→ — ∓5 с, J/L — ∓10 с,
//! ↑/↓ — громкость, M — звук, F — во весь экран, 0–9 — доля ролика.
//!
//! Фоновый поток-тикер (как `install_autohide` в tv_rezka) прячет панель,
//! ловит конец ролика и доводит отложенную перемотку. Плеер он держит слабой
//! ссылкой, а живёт, пока жив «жетон» в обработчике клавиш: пересобрали
//! виджет или закрыли просмотр — поток завершается сам.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use syngui::core::sync::Mutex;
use syngui::input::{CursorIcon, Key, Modifiers};
use syngui::prelude::*;
use syngui::video::VideoPlayer;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::visual::VideoView;

use crate::components::event_hook::{EventHook, KeyReply};
use crate::icons::{
    MI_FORWARD_10, MI_FULLSCREEN, MI_FULLSCREEN_EXIT, MI_PAUSE, MI_PLAY_ARROW, MI_REPLAY,
    MI_REPLAY_10, MI_VOLUME_DOWN, MI_VOLUME_OFF, MI_VOLUME_UP,
};
use crate::syn_chat::attach;

/// Сколько панель держится без движения мыши при воспроизведении.
const HIDE_AFTER: Duration = Duration::from_millis(2500);
/// Курсор ушёл с плеера — панель прячется через столько.
const LEAVE_GRACE: Duration = Duration::from_millis(600);
/// Шаг тикера: отложенная перемотка и автоскрытие.
const TICK: Duration = Duration::from_millis(50);
/// Перемотка при перетаскивании ползунка — не чаще раза в столько: каждый
/// `seek` пересоздаёт аудиовывод, и на каждое движение мыши звук заикался бы.
const SEEK_THROTTLE: Duration = Duration::from_millis(90);
/// После последнего движения ползунка он ещё столько показывает цель, а не
/// часы плеера — те догоняют перемотку не сразу.
const SCRUB_SETTLE: Duration = Duration::from_millis(250);
/// Шаги перемотки: стрелки и кнопки/J/L.
const ARROW_STEP: f64 = 5.0;
const BUTTON_STEP: f64 = 10.0;
const VOLUME_STEP: f32 = 0.1;

/// Громкость между роликами: следующий открывается с той же, а «Включить
/// звук» возвращает уровень до выключения.
static LAST_VOLUME: AtomicU32 = AtomicU32::new(0x3F80_0000); // 1.0
static UNMUTE_VOLUME: AtomicU32 = AtomicU32::new(0x3F80_0000);

/// Кнопка «во весь экран»: состояние и переключатель — у владельца плеера
/// (он меняет раскладку вокруг и режим окна).
#[derive(Clone)]
pub struct FullscreenCtl {
    pub active: bool,
    pub toggle: Arc<dyn Fn() + Send + Sync>,
}

/// Состояние, которое читает поток-тикер: сигналы syngui живут в
/// thread-local рантайме главного потока, из фона их можно только `set`.
struct Shared {
    last_activity: std::sync::Mutex<Instant>,
    /// Зеркало сигнала `visible`.
    shown: AtomicBool,
    /// Курсор над панелью — не прятать её из-под руки.
    over_controls: AtomicBool,
    scrub: std::sync::Mutex<Scrub>,
}

/// Перетаскивание ползунка и перемотка клавишами.
#[derive(Default)]
struct Scrub {
    /// Цель, до которой ещё не дошли из-за [`SEEK_THROTTLE`].
    pending: Option<f64>,
    last_seek: Option<Instant>,
    last_change: Option<Instant>,
    /// Ползунок показывает `scrub`, а не позицию плеера.
    active: bool,
}

#[derive(Clone)]
struct Ctl {
    player: Arc<Mutex<VideoPlayer>>,
    shared: Arc<Shared>,
    duration: f64,
    pos: RwSignal<f32>,
    paused: RwSignal<bool>,
    ended: RwSignal<bool>,
    visible: RwSignal<bool>,
    volume: RwSignal<f32>,
    /// Цель перемотки, пока её тащат (см. [`Scrub::active`]).
    scrub: RwSignal<Option<f32>>,
    fullscreen: Option<FullscreenCtl>,
}

/// Плеер на всю отведённую площадь. Громкость берётся с прошлого ролика.
pub fn video_player(
    player: Arc<Mutex<VideoPlayer>>,
    fullscreen: Option<FullscreenCtl>,
) -> impl Widget {
    let (duration, paused, pos) = player
        .lock()
        .map(|p| (p.duration_sec().max(0.0), p.is_paused(), p.position_sec()))
        .unwrap_or((0.0, true, 0.0));
    let volume = f32::from_bits(LAST_VOLUME.load(Ordering::Relaxed));
    if let Ok(p) = player.lock() {
        p.set_volume(volume);
    }

    let ctl = Ctl {
        player: player.clone(),
        shared: Arc::new(Shared {
            last_activity: std::sync::Mutex::new(Instant::now()),
            shown: AtomicBool::new(true),
            over_controls: AtomicBool::new(false),
            scrub: std::sync::Mutex::new(Scrub::default()),
        }),
        duration,
        pos: use_signal(pos.min(duration) as f32),
        paused: use_signal(paused),
        ended: use_signal(false),
        visible: use_signal(true),
        volume: use_signal(volume),
        scrub: use_signal(None),
        fullscreen,
    };
    let token = Arc::new(());
    spawn_ticker(&ctl, Arc::downgrade(&token));

    let video = {
        let click = ctl.clone();
        let dbl = ctl.clone();
        GestureDetector::new()
            .cursor(CursorIcon::Default)
            .on_click(move || click.toggle())
            .on_double_click(move || dbl.toggle_fullscreen())
            .child(
                VideoView::new(player)
                    .fit(ImageFit::Contain)
                    .position_signal(ctl.pos)
                    .class("vp-canvas"),
            )
    };

    let stage = Stack::new()
        .fit(StackFit::Expand)
        .child(video)
        .child(center_badge(ctl.clone()))
        .child(controls_layer(ctl.clone()));

    let keys = ctl.clone();
    let captured = ctl.clone();
    let mover = ctl.clone();
    EventHook::new()
        .on_key_down(move |key, mods| {
            // Жетон живёт в обработчике: элемент удалили — поток-тикер
            // видит мёртвую слабую ссылку и выходит.
            let _alive = &token;
            keys.on_key(key, mods)
        })
        .capture_keys(move |key| captured.handles_key(key))
        .on_mouse_move(move |inside| {
            if inside {
                mover.wake();
            } else {
                mover.leave();
            }
        })
        .child(DecoratedBox::new().class("vp").child(stage))
}

// ─────────────────────────────────────────────────────────────────────────────
// Слои поверх кадра
// ─────────────────────────────────────────────────────────────────────────────

/// Большая ⏵ по центру на паузе и ⟲ в конце ролика. Колонка — прямой
/// ребёнок `Stack` и растянута им на всю сцену (`Reactive` отдаёт ребёнку
/// свободные ограничения — внутри него колонка сжалась бы к верху). Мимо
/// кнопки колонка пропускает клики к кадру под ней.
fn center_badge(ctl: Ctl) -> impl Widget {
    let badge = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ended = ctl.ended.get();
        if !ctl.paused.get() && !ended {
            return vec![];
        }
        let c = ctl.clone();
        let (icon, tip) = if ended {
            (MI_REPLAY, tr!("video_player.replay.tooltip"))
        } else {
            (MI_PLAY_ARROW, tr!("video_player.play.tooltip"))
        };
        vec![Box::new(
            ToolButton::new(icon)
                .tooltip(tip)
                .on_click(move || c.toggle())
                .class("vp-big-play"),
        )]
    });
    Column::new()
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(badge)
}

/// Панель у нижнего края. Как и у [`center_badge`], колонка снаружи
/// `Reactive`; `End` без распорки — над панелью клики проходят к кадру
/// (распорка-`DecoratedBox` перехватила бы их).
fn controls_layer(ctl: Ctl) -> impl Widget {
    let panel = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let shown = ctl.visible.get() || ctl.paused.get();
        if !shown {
            return vec![];
        }
        let hover = ctl.clone();
        vec![Box::new(
            GestureDetector::new()
                .cursor(CursorIcon::Default)
                .on_hover_change(move |over| {
                    hover.shared.over_controls.store(over, Ordering::Relaxed);
                    if over {
                        hover.wake();
                    }
                })
                .child(
                    DecoratedBox::new().class("vp-controls").child(
                        Column::new()
                            .gap(6.0)
                            .cross_axis_alignment(CrossAxisAlignment::Stretch)
                            .child(seek_bar(ctl.clone()))
                            .child(bottom_row(ctl.clone())),
                    ),
                ),
        )]
    });
    Column::new()
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(panel)
}

fn seek_bar(ctl: Ctl) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let value = ctl.scrub.get().unwrap_or_else(|| ctl.pos.get());
        let dur = ctl.duration as f32;
        let c = ctl.clone();
        vec![Box::new(
            Slider::new()
                .value(value.clamp(0.0, dur.max(0.0)))
                .range(0.0, dur.max(0.1))
                .step(0.05)
                .on_change(move |t: f32| c.request_seek(t as f64))
                .class("vp-seek"),
        )]
    })
}

/// Время слева, транспорт строго по центру, звук и экран справа — три ряда
/// во всю ширину в одном `Stack` (как в tv_rezka): центр не съезжает от
/// ширины боковых групп, а пустые места рядов пропускают клики к соседям.
fn bottom_row(ctl: Ctl) -> impl Widget {
    let left = Row::new()
        .main_axis_alignment(MainAxisAlignment::Start)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(time_label(ctl.clone()))
        .class("vp-row");

    let mut right_items: Vec<Box<dyn Widget>> = vec![
        Box::new(mute_button(ctl.clone())),
        Box::new(volume_slider(ctl.clone())),
    ];
    if let Some(fs) = ctl.fullscreen.clone() {
        let c = ctl.clone();
        let (icon, tip) = if fs.active {
            (
                MI_FULLSCREEN_EXIT,
                tr!("video_player.exit_fullscreen.tooltip"),
            )
        } else {
            (MI_FULLSCREEN, tr!("video_player.fullscreen.tooltip"))
        };
        right_items.push(Box::new(
            ToolButton::new(icon)
                .tooltip(tip)
                .on_click(move || c.toggle_fullscreen())
                .class("vp-btn"),
        ));
    }
    let right = Row::new()
        .gap(4.0)
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(right_items)
        .class("vp-row");

    let back = ctl.clone();
    let fwd = ctl.clone();
    let center = Row::new()
        .gap(14.0)
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            ToolButton::new(MI_REPLAY_10)
                .tooltip(tr!("video_player.back.tooltip"))
                .on_click(move || back.seek_by(-BUTTON_STEP))
                .class("vp-btn"),
        )
        .child(play_button(ctl.clone()))
        .child(
            ToolButton::new(MI_FORWARD_10)
                .tooltip(tr!("video_player.forward.tooltip"))
                .on_click(move || fwd.seek_by(BUTTON_STEP))
                .class("vp-btn"),
        )
        .class("vp-row");

    Stack::new().child(left).child(right).child(center)
}

/// Отдельный `Reactive` на кнопку: позиция тикает много раз в секунду, и
/// общий блок пересобирал бы кнопку между press и release — клик терялся
/// (та же история, что с ⏵ аудио в `media_viewer`).
fn play_button(ctl: Ctl) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let paused = ctl.paused.get();
        let ended = ctl.ended.get();
        let (icon, tip) = match (ended, paused) {
            (true, _) => (MI_REPLAY, tr!("video_player.replay.tooltip")),
            (false, true) => (MI_PLAY_ARROW, tr!("video_player.play.tooltip")),
            (false, false) => (MI_PAUSE, tr!("video_player.pause.tooltip")),
        };
        let c = ctl.clone();
        vec![Box::new(
            ToolButton::new(icon)
                .tooltip(tip)
                .on_click(move || c.toggle())
                .class("vp-play"),
        )]
    })
}

fn time_label(ctl: Ctl) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let cur = ctl.scrub.get().unwrap_or_else(|| ctl.pos.get()) as f64;
        let cur = cur.clamp(0.0, ctl.duration);
        let text = format!(
            "{} / {}",
            attach::format_duration((cur * 1000.0) as u64),
            attach::format_duration((ctl.duration * 1000.0) as u64)
        );
        vec![Box::new(Text::new(text).class("vp-time"))]
    })
}

fn mute_button(ctl: Ctl) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let v = ctl.volume.get();
        let (icon, tip) = if v <= 0.0 {
            (MI_VOLUME_OFF, tr!("video_player.unmute.tooltip"))
        } else if v < 0.5 {
            (MI_VOLUME_DOWN, tr!("video_player.mute.tooltip"))
        } else {
            (MI_VOLUME_UP, tr!("video_player.mute.tooltip"))
        };
        let c = ctl.clone();
        vec![Box::new(
            ToolButton::new(icon)
                .tooltip(tip)
                .on_click(move || c.toggle_mute())
                .class("vp-btn"),
        )]
    })
}

fn volume_slider(ctl: Ctl) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let v = ctl.volume.get();
        let c = ctl.clone();
        vec![Box::new(
            Slider::new()
                .value(v)
                .range(0.0, 1.0)
                .step(0.01)
                .width(88.0)
                .on_change(move |nv: f32| c.set_volume(nv))
                .class("vp-volume"),
        )]
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Действия
// ─────────────────────────────────────────────────────────────────────────────

impl Ctl {
    /// Активность пользователя: панель показать и отложить автоскрытие.
    fn wake(&self) {
        if let Ok(mut t) = self.shared.last_activity.lock() {
            *t = Instant::now();
        }
        self.shared.shown.store(true, Ordering::Relaxed);
        self.visible.set(true);
    }

    /// Курсор ушёл с плеера: при воспроизведении панель прячется почти
    /// сразу, а не через полный [`HIDE_AFTER`].
    fn leave(&self) {
        if let Ok(mut t) = self.shared.last_activity.lock() {
            if let Some(early) = Instant::now().checked_sub(HIDE_AFTER - LEAVE_GRACE) {
                *t = (*t).min(early);
            }
        }
    }

    fn toggle(&self) {
        self.wake();
        let restart = self.ended.get_untracked();
        let now_paused = {
            let Ok(mut p) = self.player.lock() else {
                return;
            };
            if restart {
                let _ = p.seek(0.0);
                p.play();
            } else if p.is_paused() {
                p.play();
            } else {
                p.pause();
            }
            p.is_paused()
        };
        if restart {
            self.ended.set(false);
            self.pos.set(0.0);
        }
        self.paused.set(now_paused);
    }

    fn toggle_fullscreen(&self) {
        if let Some(fs) = &self.fullscreen {
            (fs.toggle)();
        }
    }

    /// Перемотка ползунком или клавишами. Первая — сразу, дальше не чаще
    /// [`SEEK_THROTTLE`]; хвост доводит тикер.
    fn request_seek(&self, target: f64) {
        self.wake();
        let target = target.clamp(0.0, self.duration);
        self.scrub.set(Some(target as f32));
        self.ended.set(false);
        let now = Instant::now();
        let seek_now = {
            let Ok(mut s) = self.shared.scrub.lock() else {
                return;
            };
            s.active = true;
            s.last_change = Some(now);
            let due = s
                .last_seek
                .is_none_or(|at| now.duration_since(at) >= SEEK_THROTTLE);
            if due {
                s.last_seek = Some(now);
                s.pending = None;
            } else {
                s.pending = Some(target);
            }
            due
        };
        if seek_now {
            seek_player(&self.player, target);
            self.pos.set(target as f32);
        }
    }

    fn seek_by(&self, delta: f64) {
        let base = self
            .scrub
            .get_untracked()
            .unwrap_or_else(|| self.pos.get_untracked()) as f64;
        self.request_seek(base + delta);
    }

    fn set_volume(&self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        if let Ok(p) = self.player.lock() {
            p.set_volume(v);
        }
        LAST_VOLUME.store(v.to_bits(), Ordering::Relaxed);
        if v > 0.0 {
            UNMUTE_VOLUME.store(v.to_bits(), Ordering::Relaxed);
        }
        self.volume.set(v);
        self.wake();
    }

    fn toggle_mute(&self) {
        if self.volume.get_untracked() > 0.0 {
            self.set_volume(0.0);
        } else {
            let back = f32::from_bits(UNMUTE_VOLUME.load(Ordering::Relaxed));
            self.set_volume(if back > 0.0 { back } else { 1.0 });
        }
    }

    fn handles_key(&self, key: Key) -> bool {
        match key {
            Key::Space
            | Key::K
            | Key::MediaPlayPause
            | Key::Left
            | Key::Right
            | Key::J
            | Key::L
            | Key::Up
            | Key::Down
            | Key::M
            | Key::Home
            | Key::Num0
            | Key::Num1
            | Key::Num2
            | Key::Num3
            | Key::Num4
            | Key::Num5
            | Key::Num6
            | Key::Num7
            | Key::Num8
            | Key::Num9 => true,
            Key::F => self.fullscreen.is_some(),
            // Esc во весь экран — выход из него; иначе Esc закрывает
            // просмотрщик (его ловит `Portal`).
            Key::Escape => self.fullscreen.as_ref().is_some_and(|f| f.active),
            _ => false,
        }
    }

    fn on_key(&self, key: Key, mods: Modifiers) -> KeyReply {
        if mods.ctrl || mods.alt || mods.meta || !self.handles_key(key) {
            return KeyReply::Ignore;
        }
        match key {
            Key::Space | Key::K | Key::MediaPlayPause => self.toggle(),
            Key::Left => self.seek_by(-ARROW_STEP),
            Key::Right => self.seek_by(ARROW_STEP),
            Key::J => self.seek_by(-BUTTON_STEP),
            Key::L => self.seek_by(BUTTON_STEP),
            Key::Up => self.set_volume(self.volume.get_untracked() + VOLUME_STEP),
            Key::Down => self.set_volume(self.volume.get_untracked() - VOLUME_STEP),
            Key::M => self.toggle_mute(),
            Key::F | Key::Escape => self.toggle_fullscreen(),
            Key::Home => self.request_seek(0.0),
            other => {
                if let Some(n) = digit(other) {
                    self.request_seek(self.duration * n as f64 / 10.0);
                }
            }
        }
        KeyReply::Handled
    }
}

fn digit(key: Key) -> Option<u8> {
    Some(match key {
        Key::Num0 => 0,
        Key::Num1 => 1,
        Key::Num2 => 2,
        Key::Num3 => 3,
        Key::Num4 => 4,
        Key::Num5 => 5,
        Key::Num6 => 6,
        Key::Num7 => 7,
        Key::Num8 => 8,
        Key::Num9 => 9,
        _ => return None,
    })
}

fn seek_player(player: &Mutex<VideoPlayer>, target: f64) {
    if let Ok(mut p) = player.lock() {
        if let Err(e) = p.seek(target) {
            log::warn!("[video-player] seek {target:.2}s: {e}");
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Тикер
// ─────────────────────────────────────────────────────────────────────────────

/// Сигнал из потока тикера — только через очередь главного потока. Сам
/// `RwSignal::set` из фона тоже маршалит туда, но полагается на
/// зарегистрированный главный поток; в `TestHarness` его нет, и `set` ушёл
/// бы в thread-local рантайм тикера, где такого слота нет.
fn set_on_main<T: PartialEq + Send + Clone + 'static>(sig: RwSignal<T>, value: T) {
    syngui::async_runtime::run_on_main_thread(move || sig.set(value));
}

/// Фоновый поток плеера: хвост перемотки, конец ролика, автоскрытие.
fn spawn_ticker(ctl: &Ctl, token: Weak<()>) {
    let player = Arc::downgrade(&ctl.player);
    let shared = ctl.shared.clone();
    let duration = ctl.duration;
    let (pos_sig, paused_sig, ended_sig, visible_sig, scrub_sig) =
        (ctl.pos, ctl.paused, ctl.ended, ctl.visible, ctl.scrub);
    let initially_paused = ctl.paused.get_untracked();
    let spawned = std::thread::Builder::new()
        .name("video-player-tick".into())
        .spawn(move || {
            let mut last_paused = initially_paused;
            let mut last_pos = f64::NAN;
            let mut last_move = Instant::now();
            let mut moved = false;
            loop {
                std::thread::sleep(TICK);
                if token.upgrade().is_none() {
                    break;
                }
                let Some(player) = player.upgrade() else {
                    break;
                };

                // Хвост перемотки: последняя цель, придержанная троттлингом.
                let (pending, settled) = match shared.scrub.lock() {
                    Ok(mut s) => {
                        let due = s.last_seek.is_none_or(|at| at.elapsed() >= SEEK_THROTTLE);
                        let pending = if due { s.pending.take() } else { None };
                        if pending.is_some() {
                            s.last_seek = Some(Instant::now());
                        }
                        let settled = s.active
                            && s.pending.is_none()
                            && s.last_change.is_some_and(|t| t.elapsed() >= SCRUB_SETTLE);
                        if settled {
                            s.active = false;
                        }
                        (pending, settled)
                    }
                    Err(_) => (None, false),
                };
                if let Some(t) = pending {
                    seek_player(&player, t);
                    set_on_main(pos_sig, t as f32);
                }
                if settled {
                    set_on_main(scrub_sig, None);
                }
                let scrubbing = shared.scrub.lock().map(|s| s.active).unwrap_or(false);

                let (paused, pos) = match player.lock() {
                    Ok(p) => (p.is_paused(), p.position_sec()),
                    Err(_) => continue,
                };
                if last_paused != paused {
                    last_paused = paused;
                    set_on_main(paused_sig, paused);
                    last_move = Instant::now();
                    moved = false;
                }

                // Конец ролика: без звука часы идут дальше длительности, со
                // звуком встают чуть раньше неё — ловим оба случая.
                if !paused && !scrubbing && duration > 0.0 {
                    if (pos - last_pos).abs() > 1e-3 {
                        if !last_pos.is_nan() {
                            moved = true;
                        }
                        last_pos = pos;
                        last_move = Instant::now();
                    }
                    let stalled = moved && last_move.elapsed() >= Duration::from_millis(700);
                    if pos >= duration - 0.05 || (pos >= duration - 1.0 && stalled) {
                        if let Ok(mut p) = player.lock() {
                            p.pause();
                        }
                        last_paused = true;
                        set_on_main(paused_sig, true);
                        set_on_main(ended_sig, true);
                        set_on_main(pos_sig, duration as f32);
                        shared.shown.store(true, Ordering::Relaxed);
                        set_on_main(visible_sig, true);
                        continue;
                    }
                }
                drop(player);

                let idle = shared
                    .last_activity
                    .lock()
                    .map(|t| t.elapsed())
                    .unwrap_or_default();
                if !paused
                    && !scrubbing
                    && idle >= HIDE_AFTER
                    && !shared.over_controls.load(Ordering::Relaxed)
                    && shared.shown.swap(false, Ordering::Relaxed)
                {
                    set_on_main(visible_sig, false);
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("[video-player] поток тикера не запустился: {e}");
    }
}
