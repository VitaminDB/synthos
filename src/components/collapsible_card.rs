//! Сворачиваемая карточка боковых панелей Syn-чата: кликабельная шапка
//! (иконка, заголовок, действия, шеврон) и тело, которое `AnimatedSize`
//! плавно раскрывает по высоте. Одна на все секции — слева «Инструменты /
//! Autotools / Скилы», справа карточки таба «Параметры», — чтобы шапки не
//! разъезжались при первой же правке.
//!
//! ```text
//! ┌──────────────────────────────────┐
//! │ [ик] ЗАГОЛОВОК      [+][✎][🗑] ˄ │  ← щелчок — свернуть/раскрыть
//! │ …тело…                           │
//! └──────────────────────────────────┘
//! ```
//!
//! Сама карточка не реактивна: шапка и тело — отдельные `Reactive`, а
//! `AnimatedSize` между ними переживает перестройку тела. Будь карточка
//! одним замыканием, щелчок пересоздавал бы и `AnimatedSize`, и высота
//! прыгала бы без анимации.
//!
//! Раскрытие хранит `SynChatCtx.cards`, на диск его пишет автосейв
//! конфига. Классы `.collapsible-card-*` — в `styles/components/syn_chat.mss`.

use std::sync::Arc;

use syngui::animation::Easing;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::{AnimatedSize, AnimationAxis, Reactive};

use crate::icons::{MI_EXPAND_LESS, MI_EXPAND_MORE};

type Rows = Arc<dyn Fn() -> Vec<Box<dyn Widget>> + Send + Sync>;

pub struct CollapsibleCard {
    class: &'static str,
    icon: &'static str,
    title: String,
    title_class: &'static str,
    open: RwSignal<bool>,
    gap: f32,
    actions: Option<Rows>,
}

impl CollapsibleCard {
    /// `class` — оформление карточки (`sampling-card` справа,
    /// `tools-section` слева), `open` — её флаг в `SynChatCtx.cards`.
    pub fn new(class: &'static str, icon: &'static str, title: String, open: RwSignal<bool>) -> Self {
        Self {
            class,
            icon,
            title,
            title_class: "right-panel-section-title",
            open,
            gap: 8.0,
            actions: None,
        }
    }

    pub fn title_class(mut self, class: &'static str) -> Self {
        self.title_class = class;
        self
    }

    /// Отступ между строками тела.
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Кнопки в шапке между заголовком и шевроном. Показываются только у
    /// раскрытой карточки: действуют они на тело.
    pub fn actions(mut self, build: impl Fn() -> Vec<Box<dyn Widget>> + Send + Sync + 'static) -> Self {
        self.actions = Some(Arc::new(build));
        self
    }

    /// Тело — строки столбцом. Замыкание живёт в `Reactive`: сигналы,
    /// прочитанные в нём, перестраивают только тело, а не шапку.
    pub fn body(self, rows: impl Fn() -> Vec<Box<dyn Widget>> + Send + Sync + 'static) -> StyledWidget<DecoratedBox> {
        let Self {
            class,
            icon,
            title,
            title_class,
            open,
            gap,
            actions,
        } = self;

        let header = Reactive::new(move || {
            let is_open = open.get();
            let mut row: Vec<Box<dyn Widget>> = vec![
                Box::new(Icon::new(icon).class("collapsible-card-icon")),
                // Заголовок — flex-элемент: кнопки и шеврон держат свой
                // размер, заголовок ужимается до остатка.
                Box::new(
                    DecoratedBox::new()
                        .class("collapsible-card-title")
                        .child(Text::new(title.clone()).max_lines(1).class(title_class)),
                ),
            ];
            if let (true, Some(actions)) = (is_open, actions.as_ref()) {
                row.push(Box::new(
                    Row::new()
                        .gap(2.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(actions()),
                ));
            }
            row.push(Box::new(
                Icon::new(if is_open { MI_EXPAND_LESS } else { MI_EXPAND_MORE }).class("collapsible-card-chevron"),
            ));
            // Кнопки действий берут нажатие сами — дети получают события
            // раньше родителя, — так что щелчок по ним карточку не сворачивает.
            vec![Box::new(
                GestureDetector::new()
                    .on_click(move || open.set(!open.get_untracked()))
                    .child(
                        Row::new()
                            .gap(8.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(row),
                    ),
            ) as Box<dyn Widget>]
        });

        let body = Reactive::new(move || {
            if !open.get() {
                return vec![Box::new(DecoratedBox::new()) as Box<dyn Widget>];
            }
            vec![Box::new(
                DecoratedBox::new().class("collapsible-card-body").child(
                    Column::new()
                        .gap(gap)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(rows()),
                ),
            ) as Box<dyn Widget>]
        });

        DecoratedBox::new().class(class).child(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![
                    Box::new(header) as Box<dyn Widget>,
                    Box::new(
                        AnimatedSize::new(body)
                            .axis(AnimationAxis::Height)
                            .duration_ms(200)
                            .easing(Easing::EaseOutCubic),
                    ),
                ]),
        )
    }
}
