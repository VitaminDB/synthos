//! Заголовки панелей трёхпанельного каркаса — одна высота и один стиль
//! на всех страницах ([`super::workspace_frame`] раскладывает их по
//! колонкам).
//!
//! ```text
//! ┌─────────────────────┬──────────────────────────────────────┬──────────────────┐
//! │ ▤ [ик] Левая панель │ [ик] Заголовок  [🔍 Поиск  Ctrl K] ⋯ │ Правая панель  ▥ │
//! └─────────────────────┴──────────────────────────────────────┴──────────────────┘
//! ```
//!
//! * боковые заголовки — контент страницы (`side_title`, заголовок с
//!   кнопкой, TabBar) плюс тоггл панели у внешнего края;
//! * центральный ([`center`]) — идентичность страницы слева, пилюля
//!   глобального поиска ([`crate::search::trigger`]) по центру, действия
//!   справа. Если боковая панель скрыта, её тоггл переезжает к
//!   соответствующему краю центрального заголовка — панель всегда можно
//!   вернуть.
//!
//! Классы: `.panel-header*` в `styles/components/panel_header.mss`.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;

use crate::icons::{MI_VERTICAL_SPLIT, MI_VIEW_SIDEBAR};
use crate::search;

/// Содержимое центрального заголовка.
pub struct CenterSpec {
    /// Блок слева: иконка + заголовок + подзаголовок. См. [`identity`].
    pub identity: Box<dyn Widget>,
    /// Дополнительный виджет справа от пилюли поиска (HF-поиск по Hub).
    pub center_extra: Option<Box<dyn Widget>>,
    /// Ряд действий справа.
    pub actions: Option<Box<dyn Widget>>,
}

impl CenterSpec {
    pub fn new(identity: Box<dyn Widget>) -> Self {
        Self {
            identity,
            center_extra: None,
            actions: None,
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
}

/// Контент центрального заголовка: идентичность | поиск (растягивается) |
/// действия. Тогглы скрытых панелей добавляет каркас.
pub fn center(spec: CenterSpec) -> impl Widget {
    let CenterSpec {
        identity,
        center_extra,
        actions,
    } = spec;

    let left = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("panel-header-left")
        .child(Stack::new().children(vec![identity]));

    let mut center = Row::new()
        .gap(10.0)
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(search::trigger::view());
    if let Some(extra) = center_extra {
        center = center.child(Stack::new().children(vec![extra]));
    }
    let center_slot = DecoratedBox::new().class("grow panel-header-center").child(center);

    let mut right = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::End)
        .class("panel-header-right");
    if let Some(a) = actions {
        right = right.child(Stack::new().children(vec![a]));
    }

    mgui! {
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                left,
                center_slot,
                right,
            ]
    }
}

/// Заголовок боковой панели: маленькая иконка + текст.
pub fn side_title(icon: &'static str, text: impl Into<String>) -> impl Widget {
    mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(icon).class("panel-header-side-icon"),
                Text::new(text).max_lines(1).class("panel-header-side-title"),
            ]
    }
}

/// Тоггл левой панели: подсвечен, пока панель открыта.
pub fn left_toggle(sig: RwSignal<bool>) -> impl Widget {
    toggle_button(sig, MI_VERTICAL_SPLIT, "header.toggle.left")
}

/// Тоггл правой панели.
pub fn right_toggle(sig: RwSignal<bool>) -> impl Widget {
    toggle_button(sig, MI_VIEW_SIDEBAR, "header.toggle.right")
}

fn toggle_button(sig: RwSignal<bool>, icon: &'static str, tooltip_key: &'static str) -> impl Widget {
    DecoratedBox::new().child(move || {
        let on = sig.get();
        let class = if on {
            "panel-header-toggle panel-header-toggle--on"
        } else {
            "panel-header-toggle"
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
            .class("panel-header-identity") => [
                leading,
                info,
            ]
    })
}

/// Идентичность из иконки Material и двух строк текста.
pub fn identity_text(icon: &'static str, title: String, subtitle: String) -> Box<dyn Widget> {
    identity(
        icon_bubble(icon),
        Text::new(title).max_lines(1).class("panel-header-title"),
        Text::new(subtitle).max_lines(1).class("panel-header-subtitle"),
    )
}

/// Круглая подложка с иконкой — визуальный аналог аватара чата.
pub fn icon_bubble(icon: &'static str) -> impl Widget {
    DecoratedBox::new()
        .class("panel-header-icon-bubble")
        .child(Center::new().child(Icon::new(icon).class("panel-header-icon")))
}

pub fn title_text(text: impl Into<String>) -> impl Widget {
    Text::new(text).max_lines(1).class("panel-header-title")
}

pub fn subtitle_text(text: impl Into<String>) -> impl Widget {
    Text::new(text).max_lines(1).class("panel-header-subtitle")
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
        .class("panel-header-action")
}

/// Тонкий вертикальный разделитель между группами действий.
pub fn action_divider() -> impl Widget {
    DecoratedBox::new().class("panel-header-action-divider")
}
