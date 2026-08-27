//! Общая шапка страницы — одна строка на всю ширину контента.
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────┐
//! │ ▤ [ик] Заголовок      [🔍 Поиск по приложению  Ctrl K] [доп]  ⋯  ▥ │
//! │        подзаголовок                                                  │
//! └──────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! * слева — тоггл левой панели (если у страницы она есть) и блок
//!   идентичности: иконка/аватар, заголовок, подзаголовок;
//! * по центру — пилюля глобального поиска ([`crate::search::trigger`]) и,
//!   при необходимости, дополнительный виджет страницы (поле поиска HF);
//! * справа — действия страницы и тоггл правой панели.
//!
//! Шапка строится страницей (у каждой свой набор действий), но выглядит
//! везде одинаково — поэтому вся разметка здесь, а страница передаёт лишь
//! [`HeaderSpec`]. Классы: `.page-header*` в `styles/components/page_header.mss`.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;

use crate::icons::{MI_VERTICAL_SPLIT, MI_VIEW_SIDEBAR};
use crate::search;

/// Что страница кладёт в шапку. Виджеты идентичности/действий могут быть
/// реактивными сами по себе (`Reactive` / `.child(|| …)`).
pub struct HeaderSpec {
    /// Блок слева: иконка + заголовок + подзаголовок. См. [`identity`].
    pub identity: Box<dyn Widget>,
    /// Дополнительный виджет справа от пилюли поиска (HF-поиск по Hub).
    pub center_extra: Option<Box<dyn Widget>>,
    /// Ряд действий справа. `None` — только тоггл правой панели.
    pub actions: Option<Box<dyn Widget>>,
    /// Сигналы видимости левой/правой панели каркаса — тогглы по краям.
    /// `None` — у страницы такой панели нет, кнопка не рисуется.
    pub left_toggle: Option<RwSignal<bool>>,
    pub right_toggle: Option<RwSignal<bool>>,
}

impl HeaderSpec {
    pub fn new(identity: Box<dyn Widget>) -> Self {
        Self {
            identity,
            center_extra: None,
            actions: None,
            left_toggle: None,
            right_toggle: None,
        }
    }

    pub fn actions(mut self, actions: impl Widget + 'static) -> Self {
        self.actions = Some(Box::new(actions));
        self
    }

    pub fn center_extra(mut self, extra: impl Widget + 'static) -> Self {
        self.center_extra = Some(Box::new(extra));
        self
    }

    pub fn toggles(mut self, left: Option<RwSignal<bool>>, right: Option<RwSignal<bool>>) -> Self {
        self.left_toggle = left;
        self.right_toggle = right;
        self
    }
}

/// Шапка целиком.
pub fn view(spec: HeaderSpec) -> impl Widget {
    let HeaderSpec {
        identity,
        center_extra,
        actions,
        left_toggle,
        right_toggle,
    } = spec;

    let mut left = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("page-header-left");
    if let Some(sig) = left_toggle {
        left = left.child(toggle_button(sig, MI_VERTICAL_SPLIT, "header.toggle.left"));
    }
    left = left.child(Stack::new().children(vec![identity]));

    let mut center = Row::new()
        .gap(10.0)
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(search::trigger::view());
    if let Some(extra) = center_extra {
        center = center.child(Stack::new().children(vec![extra]));
    }
    let center_slot = DecoratedBox::new().class("grow page-header-center").child(center);

    let mut right = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::End)
        .class("page-header-right");
    if let Some(a) = actions {
        right = right.child(Stack::new().children(vec![a]));
    }
    if let Some(sig) = right_toggle {
        right = right.child(toggle_button(sig, MI_VIEW_SIDEBAR, "header.toggle.right"));
    }

    DecoratedBox::new().class("page-header").child(mgui! {
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                left,
                center_slot,
                right,
            ]
    })
}

/// Тоггл боковой панели: подсвечен, пока панель открыта.
fn toggle_button(sig: RwSignal<bool>, icon: &'static str, tooltip_key: &'static str) -> impl Widget {
    DecoratedBox::new().child(move || {
        let on = sig.get();
        let class = if on {
            "page-header-toggle page-header-toggle--on"
        } else {
            "page-header-toggle"
        };
        ToolButton::new(icon)
            .tooltip(tr!(tooltip_key))
            .on_click(move || sig.set(!sig.get_untracked()))
            .class(class)
    })
}

/// Блок идентичности: любой виджет-иконка слева, заголовок и подзаголовок
/// столбиком справа. Заголовок/подзаголовок — виджеты, чтобы страница
/// могла подставить inline-переименование или реактивный текст.
pub fn identity(
    leading: impl Widget + 'static,
    title: impl Widget + 'static,
    subtitle: impl Widget + 'static,
) -> Box<dyn Widget> {
    let info = Column::new()
        .gap(1.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(title)
        .child(subtitle);
    Box::new(mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("page-header-identity") => [
                leading,
                info,
            ]
    })
}

/// Идентичность из иконки Material и двух строк текста.
pub fn identity_text(icon: &'static str, title: String, subtitle: String) -> Box<dyn Widget> {
    identity(
        icon_bubble(icon),
        Text::new(title).max_lines(1).class("page-header-title"),
        Text::new(subtitle).max_lines(1).class("page-header-subtitle"),
    )
}

/// Круглая подложка с иконкой страницы — визуальный аналог аватара чата.
pub fn icon_bubble(icon: &'static str) -> impl Widget {
    DecoratedBox::new()
        .class("page-header-icon-bubble")
        .child(Center::new().child(Icon::new(icon).class("page-header-icon")))
}

/// Заголовок шапки как простой текст.
pub fn title_text(text: impl Into<String>) -> impl Widget {
    Text::new(text).max_lines(1).class("page-header-title")
}

/// Подзаголовок шапки как простой текст.
pub fn subtitle_text(text: impl Into<String>) -> impl Widget {
    Text::new(text).max_lines(1).class("page-header-subtitle")
}

/// Кнопка действия в правом кластере — единый вид на всех страницах.
pub fn action_button(
    icon: &'static str,
    tooltip: impl Into<String>,
    on_click: impl FnMut() + Send + 'static,
) -> StyledWidget<ToolButton> {
    ToolButton::new(icon)
        .tooltip(tooltip)
        .on_click(on_click)
        .class("page-header-action")
}

/// Тонкий вертикальный разделитель между группами действий.
pub fn action_divider() -> impl Widget {
    DecoratedBox::new().class("page-header-action-divider")
}
