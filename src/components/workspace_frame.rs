//! Трёхпанельный каркас страницы: `[левая | центр | правая]` под общей
//! шапкой ([`super::page_header`]).
//!
//! ```text
//! Column [
//!   header
//!   body (Reactive по visible-сигналам) {
//!     SplitView(left, SplitView(center, right))   — обе панели видны
//!     SplitView(left, center)                     — правая скрыта
//!     SplitView(center, right)                    — левая скрыта
//!     center                                      — обе скрыты / панелей нет
//!   }
//! ]
//! ```
//!
//! Боковые панели строятся лениво (`Pane::build`) и только когда видимы:
//! скрытая панель не занимает ни места, ни памяти. Положения разделителей
//! — сигналы страницы (persist в `AppConfig.*_split_ratio`), видимость —
//! `context::PanelsCtx`.

use syngui::prelude::*;
use syngui::widgets::{SplitDirection, SplitView};

/// Боковая панель каркаса.
pub struct Pane {
    /// Строитель содержимого — зовётся при каждой пересборке тела, пока
    /// панель видима.
    pub build: Box<dyn Fn() -> Box<dyn Widget> + Send + Sync>,
    /// Положение разделителя между этой панелью и центром.
    pub ratio: RwSignal<f32>,
    /// Показывать ли панель (тоггл в шапке).
    pub visible: RwSignal<bool>,
    /// Минимальная ширина обеих сторон разделителя, px.
    pub min_size: f32,
}

impl Pane {
    pub fn new(
        visible: RwSignal<bool>,
        ratio: RwSignal<f32>,
        min_size: f32,
        build: impl Fn() -> Box<dyn Widget> + Send + Sync + 'static,
    ) -> Self {
        Self {
            build: Box::new(build),
            ratio,
            visible,
            min_size,
        }
    }
}

/// Описание тела страницы. `split_class` — MSS-класс разделителей
/// (`.syn-chat-h-split`, `.code-editor-h-split`, …): цвет линии и accent.
pub struct FrameSpec {
    pub split_class: &'static str,
    pub left: Option<Pane>,
    pub center: Box<dyn Fn() -> Box<dyn Widget> + Send + Sync>,
    pub right: Option<Pane>,
}

impl FrameSpec {
    pub fn new(
        split_class: &'static str,
        center: impl Fn() -> Box<dyn Widget> + Send + Sync + 'static,
    ) -> Self {
        Self {
            split_class,
            left: None,
            center: Box::new(center),
            right: None,
        }
    }

    pub fn left(mut self, pane: Pane) -> Self {
        self.left = Some(pane);
        self
    }

    pub fn right(mut self, pane: Pane) -> Self {
        self.right = Some(pane);
        self
    }
}

/// Каркас: шапка сверху, тело под ней на всю оставшуюся высоту.
pub fn view(header: impl Widget + 'static, spec: FrameSpec) -> impl Widget {
    let body = DecoratedBox::new()
        .class("grow workspace-body")
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> { vec![build_body(&spec)] }));
    Column::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("workspace-frame")
        .child(header)
        .child(body)
}

fn build_body(spec: &FrameSpec) -> Box<dyn Widget> {
    // `.get()` на visible — подписка Reactive'а: тоггл перестраивает тело.
    let left_on = spec.left.as_ref().map(|p| p.visible.get()).unwrap_or(false);
    let right_on = spec.right.as_ref().map(|p| p.visible.get()).unwrap_or(false);

    let center = (spec.center)();
    let with_right: Box<dyn Widget> = match (&spec.right, right_on) {
        (Some(pane), true) => Box::new(
            SplitView::new(expand(center), expand((pane.build)()))
                .class(spec.split_class)
                .direction(SplitDirection::Horizontal)
                .ratio_signal(pane.ratio)
                .min_size(pane.min_size)
                .divider_width(6.0),
        ),
        _ => center,
    };
    match (&spec.left, left_on) {
        (Some(pane), true) => Box::new(
            SplitView::new(expand((pane.build)()), expand(with_right))
                .class(spec.split_class)
                .direction(SplitDirection::Horizontal)
                .ratio_signal(pane.ratio)
                .min_size(pane.min_size)
                .divider_width(6.0),
        ),
        _ => with_right,
    }
}

/// `Box<dyn Widget>` сам по себе не `Widget` — оборачиваем в Stack на всю
/// доступную площадь (тот же приём, что в `pages::settings::right_panel`).
pub fn expand(w: Box<dyn Widget>) -> Stack {
    Stack::new().fit(StackFit::Expand).children(vec![w])
}
