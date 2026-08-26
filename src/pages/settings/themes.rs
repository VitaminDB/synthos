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
use crate::icons::{MI_BLUR_ON, MI_DESKTOP_WINDOWS, MI_PALETTE, MI_TUNE};

use super::general::{row_frame, switch_row};
use super::theme_data::{self, SynthosTheme};

pub fn view() -> impl Widget {
    let themes = theme_data::builtin_themes();

    let content = Column::new()
        .gap(24.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(tr!("settings.themes.title")).class("settings-page-title"))
        .child(
            Text::new(tr!("settings.themes.subtitle"))
                .class("settings-page-subtitle"),
        )
        .child(system_section())
        .child(section(tr!("settings.themes.section.light"), &themes, false))
        .child(section(tr!("settings.themes.section.dark"), &themes, true));

    mgui! {
        ScrollView::new().vertical() => [
            Padding::all(32.0) => [
                DecoratedBox::new().class("settings-page").child(content),
            ]
        ]
    }
}


/// Настройки, привязывающие оформление приложения к рабочему столу.
///
/// Светлая/тёмная схема и акцент приходят из XDG-портала, вид кнопок — из темы
/// декораций, размытие — от композитора. Всё, что система не сообщает, просто
/// остаётся за приложением.
fn system_section() -> impl Widget {
    let ctx = use_context::<AppCtx>();
    let a = ctx.appearance;

    let rows: Vec<Box<dyn Widget>> = vec![
        switch_row(
            MI_PALETTE,
            tr!("settings.themes.follow_system"),
            tr!("settings.themes.follow_system.desc"),
            a.follow_system,
        ),
        switch_row(
            MI_TUNE,
            tr!("settings.themes.system_accent"),
            tr!("settings.themes.system_accent.desc"),
            a.use_system_accent,
        ),
        switch_row(
            MI_DESKTOP_WINDOWS,
            tr!("settings.themes.system_window_controls"),
            tr!("settings.themes.system_window_controls.desc"),
            a.system_window_controls,
        ),
        blur_row(a.window_blur, a.window_opacity),
        opacity_row(a.window_opacity),
    ];

    mgui! {
        Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            Text::new(tr!("settings.themes.section.system")).class("settings-section-title"),
            Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(rows),
        ]
    }
}

/// Размытие само по себе не видно, пока панели непрозрачны: размывать нечего.
/// Поэтому при включении подтягиваем непрозрачность к «стеклянной», если
/// пользователь её ещё не трогал.
fn blur_row(blur: RwSignal<bool>, opacity: RwSignal<f32>) -> Box<dyn Widget> {
    let control: Box<dyn Widget> = Box::new(
        Toggle::with_state(blur.get_untracked()).on_change(move |on| {
            if on && opacity.get_untracked() > 0.95 {
                opacity.set(0.8);
            }
            blur.set(on);
        }),
    );
    row_frame(
        MI_BLUR_ON,
        tr!("settings.themes.window_blur"),
        tr!("settings.themes.window_blur.desc"),
        control,
    )
}

/// Прозрачность фоновых поверхностей. Ниже 0.6 интерфейс перестаёт читаться
/// поверх пёстрых обоев, поэтому диапазон ограничен.
fn opacity_row(value: RwSignal<f32>) -> Box<dyn Widget> {
    let control: Box<dyn Widget> = Box::new(
        Slider::new()
            .range(0.6, 1.0)
            .step(0.01)
            .value(value.get_untracked())
            .on_change(move |v| value.set((v * 100.0).round() / 100.0))
            .width(180.0),
    );
    row_frame(
        MI_TUNE,
        tr!("settings.themes.window_opacity"),
        tr!("settings.themes.window_opacity.desc"),
        control,
    )
}

/// Одна секция — «Светлые» или «Тёмные». Карточки строятся inline:
/// `DecoratedBox::new().class("grow").child(<closure>)`. Замыкание реактивно читает
/// `theme_mss` и перестраивает карточку при смене темы.
fn section(title: impl Into<String>, themes: &[SynthosTheme], dark: bool) -> impl Widget {
    let mut row = Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch);

    for t in themes.iter().filter(|t| t.is_dark == dark) {
        let mss = t.to_mss();
        let id = t.id.to_string();
        let name = t.name.to_string();
        let swatch_hexes: Vec<&'static str> = t.swatches().to_vec();

        row = row.child(DecoratedBox::new().class("grow").child(move || {
            let ctx = use_context::<AppCtx>();
            let a = ctx.appearance;
            // В системном режиме карточки задают пару: светлая тема — для
            // светлой схемы, тёмная — для тёмной.
            let is_active = if a.follow_system.get() {
                let key = if dark { a.theme_dark.get() } else { a.theme_light.get() };
                theme_data::find_or_default(&key, dark).id == id
            } else {
                ctx.theme_key.get() == id
            };
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
                            .child(Text::new(tr!("settings.themes.card.active")).class("theme-card-badge-text")),
                    ),
                )
            } else {
                let mss_click = mss.clone();
                let id_click = id.clone();
                Box::new(
                    Button::new(tr!("app.apply"))
                        .on_click(move || {
                            let ctx = use_context::<AppCtx>();
                            let a = ctx.appearance;
                            if a.follow_system.get() {
                                // Эффект в build_context сам пересоберёт MSS,
                                // когда сменится ключ нужной половины пары.
                                if dark {
                                    a.theme_dark.set(id_click.clone());
                                } else {
                                    a.theme_light.set(id_click.clone());
                                }
                            } else {
                                ctx.theme_key.set(id_click.clone());
                                ctx.theme_mss.set(mss_click.clone());
                            }
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
            Text::new(title.into()).class("settings-section-title"),
            row,
        ]
    }
}
