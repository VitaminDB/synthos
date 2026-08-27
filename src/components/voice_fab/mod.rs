//! Глобальный голосовой FAB и overlay-окно распознавания.
//!
//! Состоит из двух Portal-слоёв:
//! 1. **FAB-кнопка** — `Portal::BottomEnd { 24,24 }`, всегда открыт, без backdrop.
//!    При клике поднимает `voice.panel_open=true` и сама временно прячется
//!    через MSS-класс `.fab-voice.opening` (scale-out + fade).
//! 2. **Окно распознавания** — `Portal::Center, modal, backdrop`. Внутри:
//!    header (title + close) → status → aura (custom Canvas) → текст → actions.
//!
//! Запись/транскрипция отдана `crate::agent::audio::voice_*` — здесь только UI.

mod actions;
mod aura;
mod central_fab;
pub mod fab_button;
mod panel;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::Stack;

/// Точка входа: накладывает FAB и Portal-окно поверх любой страницы.
/// Сам Portal уходит в overlay-слой движка, поэтому стек тут существует
/// только для группировки двух виджетов в один Widget без layout-влияния.
pub fn view() -> impl Widget {
    mgui! {
        Stack::new().clip(false) => [
            fab_button::view(),
            panel::view(),
        ]
    }
}
