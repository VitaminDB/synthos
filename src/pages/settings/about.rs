use syngui::mgui;
use syngui::open_url;
use syngui::prelude::*;
use syngui::widgets::GestureDetector;
use syngui::widgets::visual::{Image, ImageFit};

use crate::icons::*;

const LOGO_SVG: &[u8] = include_bytes!("../../../packaging/synthos.svg");

pub fn view() -> impl Widget {
    mgui! {
        ScrollView::new().vertical() => [
            Padding::all(32.0) => [
                DecoratedBox::new().class("settings-page") => [
                    Column::new().gap(24.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                        Text::new("О программе").class("settings-page-title"),
                        Text::new("Информация о приложении и его авторстве.").class("settings-page-subtitle"),

                        identity_card(),

                        section("Авторы", vec![
                            text_row(MI_PERSON, "Разработка", env!("CARGO_PKG_AUTHORS")),
                        ]),

                        section("Лицензия", vec![
                            text_row(MI_DESCRIPTION, "Условия распространения", env!("CARGO_PKG_LICENSE")),
                            text_row(MI_INFO, "Copyright", "© 2024-2026 Synthos"),
                        ]),

                        section("Ссылки", vec![
                            link_row(MI_CODE, "Исходный код", "https://github.com/vitamindb/synthos"),
                            link_row(MI_BOOK, "Лицензия Apache-2.0", "https://www.apache.org/licenses/LICENSE-2.0"),
                            link_row(MI_BOOK, "Лицензия MIT", "https://opensource.org/licenses/MIT"),
                        ]),
                    ]
                ]
            ]
        ]
    }
}

fn identity_card() -> impl Widget {
    DecoratedBox::new().class("settings-card about-identity-card").child(mgui! {
        Row::new().gap(20.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            DecoratedBox::new().class("about-identity-logo").child(
                Image::from_bytes("about-page-logo", LOGO_SVG.to_vec()).fit(ImageFit::Contain),
            ),
            DecoratedBox::new().class("grow").child(mgui! {
                Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                    Text::new("Synthos").class("about-identity-name"),
                    Text::new(format!("Версия {}", env!("CARGO_PKG_VERSION"))).class("about-identity-version"),
                    Text::new("Локальный десктоп-чат с llama.cpp и голосовым вводом").class("about-identity-tagline"),
                ]
            }),
        ]
    })
}

fn section(title: &'static str, rows: Vec<Box<dyn Widget>>) -> impl Widget {
    let card = DecoratedBox::new().class("settings-card").child(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    );

    Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(title).class("settings-section-title"))
        .child(card)
}

fn row_frame(
    icon: &'static str,
    title: &'static str,
    value: String,
    trailing: Option<Box<dyn Widget>>,
) -> Box<dyn Widget> {
    let mut inner = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            DecoratedBox::new()
                .class("settings-row-icon-wrap")
                .child(Center::new().child(Icon::new(icon).class("settings-row-icon"))),
        )
        .child(
            DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(Text::new(title).class("settings-row-title"))
                    .child(Text::new(value).class("settings-row-desc")),
            ),
        );

    if let Some(t) = trailing {
        inner = inner.children(vec![t]);
    }

    Box::new(DecoratedBox::new().class("settings-row").child(inner))
}

fn text_row(icon: &'static str, title: &'static str, value: impl Into<String>) -> Box<dyn Widget> {
    row_frame(icon, title, value.into(), None)
}

fn link_row(icon: &'static str, title: &'static str, url: &'static str) -> Box<dyn Widget> {
    let trailing: Box<dyn Widget> = Box::new(
        GestureDetector::new()
            .on_click(move || {
                let _ = open_url(url);
            })
            .child(
                DecoratedBox::new()
                    .class("about-link-button")
                    .child(Center::new().child(Icon::new(MI_LAUNCH).class("about-link-icon"))),
            ),
    );
    row_frame(icon, title, url.to_string(), Some(trailing))
}
