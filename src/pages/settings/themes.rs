//! Страница выбора темы — сетка карточек со светлыми и тёмными темами.
//!
//! Верхний уровень строится **статически**, иначе ScrollView не получает
//! intrinsic-размера от Reactive-контента. Реактивна только карточка —
//! замыкание читает `theme_mss` и пересобирает её класс + кнопку /
//! бейдж в зависимости от активности темы.

use syngui::core::Color;
use syngui::mgui;
use syngui::prelude::*;

use crate::context::AppCtx;

use super::theme_data::{self, SynthosTheme};

pub fn view() -> impl Widget {
    let themes = theme_data::builtin_themes();

    let content = Column::new()
        .gap(24.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new("Темы").class("settings-page-title"))
        .child(
            Text::new("Выберите оформление приложения. Изменения применяются сразу.")
                .class("settings-page-subtitle"),
        )
        .child(section("Светлые", &themes, false))
        .child(section("Тёмные", &themes, true));

    mgui! {
        ScrollView::new().vertical() => [
            Padding::all(32.0) => [
                DecoratedBox::new().class("settings-page").child(content),
            ]
        ]
    }
}

/// Одна секция — «Светлые» или «Тёмные». Карточки строятся inline:
/// `DecoratedBox::new().class("grow").child(<closure>)`. Замыкание реактивно читает
/// `theme_mss` и перестраивает карточку при смене темы.
fn section(title: &'static str, themes: &[SynthosTheme], dark: bool) -> impl Widget {
    let mut row = Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch);

    for t in themes.iter().filter(|t| t.is_dark == dark) {
        let mss = t.to_mss();
        let id = t.id.to_string();
        let name = t.name.to_string();
        let swatch_hexes: Vec<&'static str> = t.swatches().to_vec();

        row = row.child(DecoratedBox::new().class("grow").child(move || {
            let ctx = use_context::<AppCtx>();
            let is_active = ctx.theme_key.get() == id;
            let class = if is_active { "theme-card active" } else { "theme-card" };

            let swatches: Vec<Box<dyn Widget>> = swatch_hexes
                .iter()
                .map(|hex| {
                    let color = Color::from_hex(hex);
                    Box::new(
                        DecoratedBox::new()
                            .class("theme-swatch")
                            .style("background-color", color),
                    ) as Box<dyn Widget>
                })
                .collect();

            let action: Box<dyn Widget> = if is_active {
                Box::new(
                    DecoratedBox::new().class("theme-card-badge").child(
                        Center::new()
                            .child(Text::new("Активна").class("theme-card-badge-text")),
                    ),
                )
            } else {
                let mss_click = mss.clone();
                let id_click = id.clone();
                Box::new(
                    Button::new("Применить")
                        .on_click(move || {
                            let ctx = use_context::<AppCtx>();
                            ctx.theme_key.set(id_click.clone());
                            ctx.theme_mss.set(mss_click.clone());
                        })
                        .class("theme-apply-btn"),
                )
            };

            DecoratedBox::new().class(class).child(
                Padding::all(16.0).child(
                    Column::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(
                            Row::new()
                                .gap(6.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .children(swatches),
                        )
                        .child(Text::new(name.clone()).class("theme-card-name"))
                        .children(vec![action]),
                ),
            )
        }));
    }

    mgui! {
        Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            Text::new(title).class("settings-section-title"),
            row,
        ]
    }
}
