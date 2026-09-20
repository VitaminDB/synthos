//! Settings → «Базы знаний» — управление RAG-коллекциями.
//!
//! - Центр (`view()`) — страница открытой коллекции стопкой секций в грамматике
//!   остальных настроек (`settings-section-title` + `settings-card` со строками):
//!   шапка ([`hero`]), источники и ход индексации ([`sources`]), документы
//!   ([`documents`]), пробный поиск ([`probe`]), модели ([`models_card`]).
//!   Без выбранной коллекции — приглашение создать первую.
//! - Правая панель ([`collections_panel`]) — список коллекций и «+».
//!
//! Состояние — в `AppCtx.kb` (см. `kb::ctx`). Модели никто не обязан грузить
//! руками: индексация, пробный поиск и `kb_search` поднимают их сами через
//! `kb::loader`; карточка «Модели» лишь показывает, что нашлось на диске и
//! что сейчас в памяти.

pub mod collections_panel;
mod documents;
mod hero;
mod models_card;
mod probe;
mod sources;
pub mod url_dialog;

use syngui::mgui;
use syngui::prelude::*;

use crate::context::AppCtx;
use crate::icons::*;
use crate::kb::ingest::source::DocSource;
use crate::kb::{loader, runner};

pub fn view() -> impl Widget {
    // Страница открылась — перечитать, где лежат модели: каталог из
    // «AI модели» могли сменить, бандл — докачать.
    loader::refresh_paths(use_context::<AppCtx>().kb.clone());

    DecoratedBox::new().class("settings-page kb-page").child(move || {
        let ctx = use_context::<AppCtx>();
        // Подписка только на id: имя и счётчики меняются в `registry` на
        // каждую правку, а пересборка страницы сбрасывала бы фокус поля имени.
        let active = ctx.kb.active_collection_id.get();
        let exists = active
            .as_ref()
            .is_some_and(|id| ctx.kb.registry.get_untracked().get(id).is_some());
        let child: Box<dyn Widget> = match active {
            Some(id) if exists => Box::new(editor(id)),
            _ => Box::new(empty_state()),
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    })
}

fn empty_state() -> impl Widget {
    mgui! {
        Center::new() => [
            Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().class("kb-empty-bubble") => [
                    Center::new().child(Icon::new(MI_MENU_BOOK).class("kb-empty-icon")),
                ],
                Text::new(tr!("settings.knowledge_base.empty.title")).class("kb-empty-title"),
                Text::new(tr!("settings.knowledge_base.empty.text")).class("kb-empty-text"),
                Button::new(tr!("settings.knowledge_base.empty.create"))
                    .leading_icon(MI_ADD)
                    .on_click(collections_panel::create_collection)
                    .class("kb-btn kb-btn-primary"),
            ]
        ]
    }
}

fn editor(collection_id: String) -> impl Widget {
    mgui! {
        ScrollView::new().vertical() => [
            Padding::all(32.0) => [
                Column::new().gap(24.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    hero::view(collection_id.clone()),
                    sources::view(collection_id.clone()),
                    documents::view(collection_id.clone()),
                    probe::view(collection_id),
                    models_card::view(),
                ]
            ]
        ]
    }
}

/// Заголовок секции + карточка — как `general::section`, но заголовок может
/// нести справа свой контрол (счётчик, фильтр).
fn section(
    title: impl Into<String>,
    trailing: Option<Box<dyn Widget>>,
    body: Box<dyn Widget>,
) -> Box<dyn Widget> {
    let mut head: Vec<Box<dyn Widget>> = vec![Box::new(
        DecoratedBox::new()
            .class("grow")
            .child(Text::new(title).class("settings-section-title")),
    )];
    head.extend(trailing);
    Box::new(
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(head),
            )
            .child(DecoratedBox::new().class("settings-card").child(body)),
    )
}

/// Квадратик с иконкой слева в строке — тот же, что у строк «Общих».
fn row_icon(icon: &str) -> Box<dyn Widget> {
    Box::new(
        DecoratedBox::new()
            .class("settings-row-icon-wrap")
            .child(Center::new().child(Icon::new(icon.to_string()).class("settings-row-icon"))),
    )
}

/// Запустить индексацию источников в коллекцию. Только с главного потока.
pub(super) fn launch(collection_id: String, sources: Vec<DocSource>) {
    let app = use_context::<AppCtx>();
    runner::start(
        app.kb.clone(),
        app.notifications.clone(),
        loader::plan(),
        collection_id,
        sources,
    );
}
