//! Анимированная «aura» — звуковая волна вокруг центрального микрофона.
//!
//! Custom Canvas-виджет, рисует каждый кадр (animated=true):
//! - 3 концентрических «дышащих» кольца с фазовым сдвигом от t,
//!   радиус, прозрачность и толщина зависят от RMS-уровня;
//! - 24 радиальных бара по окружности (snapshot_bars(24));
//! - центральная точка-микрофон (заливка accent-color).
//!
//! Когда запись не идёт (`vis_handle = None`), аура тихо «дышит» — рисуются
//! только три кольца с `pl=0.0`, чтобы окно не выглядело пустым.

use std::sync::{Arc, Mutex};

use syngui::audio::VisHandle;
use syngui::core::Color;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::visual::canvas::Canvas;

use crate::context::AppCtx;

/// Ширина/высота canvas'а ауры. Должна совпадать с `.voice-aura-wrap` и
/// `.voice-fab-slot` в MSS. Центральная FAB-кнопка (96×96) занимает
/// inner-зону; кольца и бары растут от `INNER_R` до внешнего радиуса,
/// который считается от размера канваса (см. `outer_radius`), — так один
/// этот параметр задаёт габарит ауры целиком.
const AURA_SIZE: f32 = 360.0;

/// Внутренний радиус колец. Привязан к FAB-кнопке: её radius 48 плюс
/// 12px воздуха, поэтому от размера канваса не зависит.
const INNER_R: f32 = 60.0;

/// Внешний радиус — до края канваса минус небольшой запас на толщину
/// штриха кольца.
fn outer_radius(w: f32, h: f32) -> f32 {
    (w.min(h) * 0.5 - 10.0).max(INNER_R + 20.0)
}

/// Реактивный wrapper: пересоздаёт Canvas при появлении / исчезновении
/// `vis_handle` (запись начата / остановлена). Когда handle — None,
/// рисуем idle-визуал с нулевым уровнем, чтобы окно не выглядело пустым.
pub fn view() -> impl Fn() -> StyledWidget<Canvas> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        // Подписка на vis_handle и is_recording — пересборка при их смене.
        let handle = app.audio.vis_handle.get();
        let _ = app.audio.is_recording.get();

        canvas_for_handle(handle).class("voice-aura")
    }
}

fn canvas_for_handle(handle: Option<VisHandle>) -> Canvas {
    let prev_bars: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(vec![0.0f32; 24]));
    let prev_level: Arc<Mutex<f32>> = Arc::new(Mutex::new(0.0));

    Canvas::new(move |ctx, t| {
        // RMS-уровень + бары. Если запись не идёт — нули, чтобы аура «спала».
        let (target, level) = match &handle {
            Some(h) => (h.snapshot_bars(24), h.level()),
            None => (vec![0.0; 24], 0.0),
        };

        // Сглаживание (lerp prev → target).
        let mut prev = prev_bars.lock().expect("aura bars lock");
        if prev.len() != target.len() {
            prev.clear();
            prev.resize(target.len(), 0.0);
        }
        for (i, v) in target.iter().enumerate() {
            prev[i] += (v - prev[i]) * 0.35;
        }
        let mut pl = prev_level.lock().expect("aura level lock");
        *pl += (level - *pl) * 0.20;

        let cx = ctx.width() * 0.5;
        let cy = ctx.height() * 0.5;
        let inner_r = INNER_R;
        let outer_r = outer_radius(ctx.width(), ctx.height());
        let accent = ctx
            .mss_accent()
            .or_else(|| ctx.mss_color())
            .unwrap_or_else(|| Color::from_hex("#3B82F6"));

        // Три пульсирующих кольца с фазовым сдвигом. При тишине — слабая
        // «дыхательная» индикация (alpha не уходит в ноль).
        for i in 0..3 {
            let phase = i as f32 / 3.0;
            let p = ((t / 2.5) + phase).fract();
            let r = inner_r + (outer_r - inner_r) * p;
            let alpha = (1.0 - p) * 0.55 * (0.4 + (*pl).clamp(0.0, 1.0));
            let stroke = 2.0 + (*pl) * 8.0;
            ctx.set_stroke_width(stroke);
            ctx.set_color(accent.with_alpha(alpha));
            ctx.stroke_circle(cx, cy, r);
        }

        // 24 радиальных бара. Стартуют чуть за внешним радиусом FAB-кнопки.
        let n = prev.len().max(1);
        for (i, v) in prev.iter().enumerate() {
            let angle = (i as f32 / n as f32) * std::f32::consts::TAU;
            let amp = v.clamp(0.0, 1.0);
            let r0 = inner_r + 8.0;
            let r1 = r0 + (outer_r - r0 - 6.0) * amp;
            let (sx, sy) = (cx + angle.cos() * r0, cy + angle.sin() * r0);
            let (ex, ey) = (cx + angle.cos() * r1, cy + angle.sin() * r1);
            ctx.set_stroke_width(4.0);
            ctx.set_color(accent);
            ctx.draw_line(sx, sy, ex, ey);
        }

        // Центральную точку-микрофон НЕ рисуем — её рисует сам ToolButton
        // (`fab-voice-center` 96×96 поверх через Stack).
    })
    .size(AURA_SIZE, AURA_SIZE)
    .animated(true)
}
