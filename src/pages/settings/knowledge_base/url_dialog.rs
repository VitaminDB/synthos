//! Модальный диалог «Добавить URL-источник» в Settings → Knowledge Base.
//!
//! Раньше [`super::prompt_url_and_ingest`] показывал только snackbar-стуб
//! «скопируйте URL в системный буфер». Этот файл реализует нормальный
//! Portal-диалог по образцу [`crate::pages::settings::skills_dialog`].
//!
//! Архитектура:
//! - Один `Portal` mount'ится в `pages::settings::view()` рядом со скилами,
//!   слушает `AppCtx.kb_url_dialog: RwSignal<Option<String>>` (collection_id).
//! - `prompt_url_and_ingest(collection_id)` теперь сводится к
//!   `kb_url_dialog.set(Some(collection_id))`.
//! - На «Добавить» делается валидация (http/https-префикс), затем вызов
//!   `super::launch(collection_id, vec![DocSource::Url(url)])`.

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::TextField;

use crate::context::AppCtx;
use crate::icons::{MI_CHECK, MI_CLOSE, MI_PUBLIC};
use crate::kb::ingest::source::DocSource;

pub fn view() -> impl Widget {
    let is_open = use_signal(false);
    create_effect(move || {
        let ctx = use_context::<AppCtx>();
        let has = ctx.kb_url_dialog.get().is_some();
        if is_open.get_untracked() != has {
            is_open.set(has);
        }
    });

    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<AppCtx>();
            let Some(collection_id) = ctx.kb_url_dialog.get() else {
                // Пустой плейсхолдер для замкнутого Reactive — Portal с
                // is_open=false всё равно не показывает детей.
                return vec![Box::new(DecoratedBox::new().class("kb-url-dialog-empty"))];
            };
            vec![Box::new(card(collection_id))]
        }))
}

fn card(collection_id: String) -> impl Widget {
    let url = use_signal(String::new());
    let error = use_signal::<Option<String>>(None);

    let confirm = {
        let collection_id = collection_id.clone();
        move || {
            let raw = url.get_untracked();
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                error.set(Some("Введите URL".into()));
                return;
            }
            if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
                error.set(Some(
                    "URL должен начинаться с http:// или https://".into(),
                ));
                return;
            }
            super::launch(
                collection_id.clone(),
                vec![DocSource::Url(trimmed.to_string())],
            );
            close_dialog();
        }
    };
    let cancel = || close_dialog();

    mgui! {
        DecoratedBox::new().class("kb-url-dialog-card") => [
            Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new("Добавить URL-источник").class("kb-url-dialog-title"),
                Text::new("Страница будет скачана, преобразована в Markdown и проиндексирована.")
                    .class("kb-url-dialog-hint"),
                TextField::new()
                    .placeholder("https://example.com/document.html")
                    .prefix_icon(MI_PUBLIC)
                    .on_change(move |s| {
                        url.set(s.to_string());
                        // Сбрасываем ошибку как только пользователь правит ввод —
                        // меньше визуального шума и подсказка «зелёная» во время
                        // активного набора.
                        if error.get_untracked().is_some() {
                            error.set(None);
                        }
                    }),
                Reactive::new(move || -> Vec<Box<dyn Widget>> {
                    match error.get() {
                        Some(msg) => vec![Box::new(
                            Text::new(msg).class("kb-url-dialog-error"),
                        ) as Box<dyn Widget>],
                        None => vec![],
                    }
                }),
                Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::End) => [
                    Button::new("Отмена")
                        .leading_icon(MI_CLOSE)
                        .on_click(cancel)
                        .class("skill-dialog-btn-secondary"),
                    Button::new("Добавить")
                        .leading_icon(MI_CHECK)
                        .on_click(confirm)
                        .class("skill-dialog-btn-primary"),
                ],
            ]
        ]
    }
}

fn close_dialog() {
    use_context::<AppCtx>().kb_url_dialog.set(None);
}
