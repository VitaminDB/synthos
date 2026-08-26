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
                        Text::new(tr!("settings.about.title")).class("settings-page-title"),
                        Text::new(tr!("settings.about.subtitle")).class("settings-page-subtitle"),

                        identity_card(),

                        section(tr!("settings.about.section.authors"), vec![
                            text_row(MI_PERSON, tr!("settings.about.development"), env!("CARGO_PKG_AUTHORS")),
                        ]),

                        section(tr!("settings.about.section.license"), vec![
                            text_row(MI_DESCRIPTION, tr!("settings.about.license_terms"), env!("CARGO_PKG_LICENSE")),
                            text_row(MI_INFO, tr!("settings.about.copyright"), "© 2024-2026 Synthos"),
                        ]),

                        section(tr!("settings.about.section.links"), vec![
                            link_row(MI_CODE, tr!("settings.about.source_code"), "https://github.com/vitamindb/synthos"),
                            link_row(MI_BOOK, tr!("settings.about.license_apache"), "https://www.apache.org/licenses/LICENSE-2.0"),
                            link_row(MI_BOOK, tr!("settings.about.license_mit"), "https://opensource.org/licenses/MIT"),
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
                    Text::new(tr!("settings.about.version", version = env!("CARGO_PKG_VERSION"))).class("about-identity-version"),
                    Text::new(tr!("settings.about.tagline")).class("about-identity-tagline"),
                ]
            }),
        ]
    })
}

fn section(title: impl Into<String>, rows: Vec<Box<dyn Widget>>) -> impl Widget {
    let card = DecoratedBox::new().class("settings-card").child(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    );

    Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(title.into()).class("settings-section-title"))
        .child(card)
}

fn row_frame(
    icon: &'static str,
    title: impl Into<String>,
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
                    .child(Text::new(title.into()).class("settings-row-title"))
                    .child(Text::new(value).class("settings-row-desc")),
            ),
        );

    if let Some(t) = trailing {
        inner = inner.children(vec![t]);
    }

    // Тот же Padding, что и в `settings::widgets::row` — `.settings-row`
    // сам по себе внутренних отступов не задаёт (только разделитель снизу),
    // и без обёртки строки «О программе» лепились вплотную к краям карточки
    // и друг к другу, в отличие от остальных страниц настроек.
    Box::new(
        DecoratedBox::new()
            .class("settings-row")
            .child(Padding::symmetric(24.0, 18.0).child(inner)),
    )
}

fn text_row(icon: &'static str, title: impl Into<String>, value: impl Into<String>) -> Box<dyn Widget> {
    row_frame(icon, title, value.into(), None)
}

fn link_row(icon: &'static str, title: impl Into<String>, url: &'static str) -> Box<dyn Widget> {
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
