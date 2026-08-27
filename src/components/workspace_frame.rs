//! Трёхпанельный каркас страницы: `[левая | центр | правая]`, у каждой
//! колонки — свой заголовок одной высоты ([`super::panel_header`]).
//!
//! ```text
//! Reactive по visible-сигналам {
//!   SplitView(left_col, SplitView(center_col, right_col))  — обе панели видны
//!   SplitView(left_col, center_col)                        — правая скрыта
//!   SplitView(center_col, right_col)                       — левая скрыта
//!   center_col                                             — обе скрыты / панелей нет
//! }
//! col = Column[ .panel-header [тоггл? | контент | тоггл?], .grow body ]
//! ```
//!
//! Тоггл левой панели стоит у левого края её заголовка, правой — у правого
//! края её заголовка; когда панель скрыта, тоггл переезжает к тому же краю
//! центрального заголовка. Боковые панели строятся лениво и только пока
//! видимы. Положения разделителей — сигналы страницы (persist в
//! `AppConfig.*_split_ratio`), видимость — `context::PanelsCtx`.

use syngui::prelude::*;
use syngui::widgets::{SplitDirection, SplitView};

use super::panel_header;

type Builder = Box<dyn Fn() -> Box<dyn Widget> + Send + Sync>;

/// Боковая панель каркаса.
pub struct Pane {
    /// Контент заголовка (без тоггла — его добавляет каркас).
    pub header: Builder,
    /// Тело панели.
    pub body: Builder,
    /// Положение разделителя между этой панелью и центром.
    pub ratio: RwSignal<f32>,
    /// Показывать ли панель (тоггл в заголовке).
    pub visible: RwSignal<bool>,
    /// Минимальная ширина обеих сторон разделителя, px.
    pub min_size: f32,
}

impl Pane {
    pub fn new(
        visible: RwSignal<bool>,
        ratio: RwSignal<f32>,
        min_size: f32,
        header: impl Fn() -> Box<dyn Widget> + Send + Sync + 'static,
        body: impl Fn() -> Box<dyn Widget> + Send + Sync + 'static,
    ) -> Self {
        Self {
            header: Box::new(header),
            body: Box::new(body),
            ratio,
            visible,
            min_size,
        }
    }
}

/// Описание страницы. `split_class` — MSS-класс разделителей
/// (`.syn-chat-h-split`, `.code-editor-h-split`, …): цвет линии и accent.
pub struct FrameSpec {
    pub split_class: &'static str,
    pub left: Option<Pane>,
    /// Контент центрального заголовка — обычно `panel_header::center(...)`.
    pub center_header: Builder,
    pub center: Builder,
    pub right: Option<Pane>,
}

impl FrameSpec {
    pub fn new(
        split_class: &'static str,
        center_header: impl Fn() -> Box<dyn Widget> + Send + Sync + 'static,
        center: impl Fn() -> Box<dyn Widget> + Send + Sync + 'static,
    ) -> Self {
        Self {
            split_class,
            left: None,
            center_header: Box::new(center_header),
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

/// Каркас целиком.
pub fn view(spec: FrameSpec) -> impl Widget {
    DecoratedBox::new()
        .class("workspace-frame")
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> { vec![build(&spec)] }))
}

fn build(spec: &FrameSpec) -> Box<dyn Widget> {
    // `.get()` на visible — подписка Reactive'а: тоггл перестраивает тело.
    let left_on = spec.left.as_ref().map(|p| p.visible.get()).unwrap_or(false);
    let right_on = spec.right.as_ref().map(|p| p.visible.get()).unwrap_or(false);

    // Тогглы скрытых панелей — в центральном заголовке, чтобы панель
    // можно было вернуть.
    let lead: Option<Box<dyn Widget>> = match &spec.left {
        Some(p) if !left_on => Some(Box::new(panel_header::left_toggle(p.visible))),
        _ => None,
    };
    let trail: Option<Box<dyn Widget>> = match &spec.right {
        Some(p) if !right_on => Some(Box::new(panel_header::right_toggle(p.visible))),
        _ => None,
    };
    let center = column(
        "panel-header--center",
        "workspace-center",
        lead,
        (spec.center_header)(),
        trail,
        (spec.center)(),
    );

    let with_right: Box<dyn Widget> = match (&spec.right, right_on) {
        (Some(pane), true) => {
            let col = column(
                "panel-header--side",
                "workspace-side workspace-right",
                None,
                (pane.header)(),
                Some(Box::new(panel_header::right_toggle(pane.visible))),
                (pane.body)(),
            );
            Box::new(
                SplitView::new(center, col)
                    .class(spec.split_class)
                    .direction(SplitDirection::Horizontal)
                    .ratio_signal(pane.ratio)
                    .min_size(pane.min_size)
                    .divider_width(6.0),
            )
        }
        _ => Box::new(center),
    };
    match (&spec.left, left_on) {
        (Some(pane), true) => {
            let col = column(
                "panel-header--side",
                "workspace-side workspace-left",
                Some(Box::new(panel_header::left_toggle(pane.visible))),
                (pane.header)(),
                None,
                (pane.body)(),
            );
            Box::new(
                SplitView::new(col, expand(with_right))
                    .class(spec.split_class)
                    .direction(SplitDirection::Horizontal)
                    .ratio_signal(pane.ratio)
                    .min_size(pane.min_size)
                    .divider_width(6.0),
            )
        }
        _ => with_right,
    }
}

/// Колонка каркаса: заголовок фиксированной высоты + тело на остаток.
fn column(
    header_class: &'static str,
    column_class: &'static str,
    leading: Option<Box<dyn Widget>>,
    content: Box<dyn Widget>,
    trailing: Option<Box<dyn Widget>>,
    body: Box<dyn Widget>,
) -> Stack {
    let mut row = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(w) = leading {
        row = row.child(Stack::new().children(vec![w]));
    }
    row = row.child(DecoratedBox::new().class("grow").child(expand(content)));
    if let Some(w) = trailing {
        row = row.child(Stack::new().children(vec![w]));
    }
    let header = DecoratedBox::new()
        .class(format!("panel-header {header_class}"))
        .child(row);
    let col = Column::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class(column_class)
        .child(header)
        .child(DecoratedBox::new().class("grow workspace-body").child(expand(body)));
    expand(Box::new(col))
}

/// `Box<dyn Widget>` сам по себе не `Widget` — оборачиваем в Stack на всю
/// доступную площадь (тот же приём, что в `pages::settings::right_panel`).
pub fn expand(w: Box<dyn Widget>) -> Stack {
    Stack::new().fit(StackFit::Expand).children(vec![w])
}
