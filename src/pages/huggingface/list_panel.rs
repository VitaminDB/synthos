//! Левая панель страницы HuggingFace: список карточек найденных моделей.
//!
//! Подписан на `models`, `list_state`, `list_error`, `selected_model`.
//! При первом открытии страницы (state == Idle) автоматически дёргает
//! `actions::trigger_list_reload` чтобы показать trending.

use syngui::mgui;
use syngui::prelude::*;

use super::{actions, card};
use super::state::{DlStatus, HfModel, HuggingFaceCtx, ListLoadState};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("hf-list-panel").child(mgui! {
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                DecoratedBox::new().class("hf-list-chips").child(super::header::sort_chips()),
                DecoratedBox::new().class("grow").child(list_body()),
            ]
    })
}

fn list_body() -> impl Widget {
    DecoratedBox::new().class("hf-list-body").child(Reactive::new(
        || -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<HuggingFaceCtx>();
            let state = ctx.list_state.get();
            // Автозагрузка trending при первом открытии страницы.
            if state == ListLoadState::Idle {
                actions::trigger_list_reload();
                return vec![Box::new(loading_skeleton())];
            }
            match state {
                ListLoadState::Loading => vec![Box::new(loading_skeleton())],
                ListLoadState::Error => vec![Box::new(error_view())],
                _ => vec![Box::new(list_view())],
            }
        },
    ))
}

fn list_view() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let models = ctx.models.get();
        let selected = ctx.selected_model.get();
        let downloads = ctx.downloads.get();

        // Репозитории с незавершённой загрузкой — закрепляем вверху списка,
        // чтобы было видно, что и откуда качается, в т.ч. после рестарта
        // (докачка возобновилась сама, а модель не в текущем поиске).
        let mut pinned: Vec<String> = downloads
            .values()
            .filter(|d| {
                matches!(
                    d.status,
                    DlStatus::Active | DlStatus::Pending | DlStatus::Paused | DlStatus::Stopped
                )
            })
            .map(|d| d.repo_id.clone())
            .collect();
        pinned.sort();
        pinned.dedup();

        if pinned.is_empty() && models.is_empty() {
            return vec![Box::new(empty_view())];
        }

        let mut col = Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch);

        if !pinned.is_empty() {
            col = col.child(Text::new(tr!("hf.list.downloading_section")).class("hf-list-section"));
            for rid in &pinned {
                let model = models
                    .iter()
                    .find(|m| &m.id == rid)
                    .cloned()
                    .unwrap_or_else(|| minimal_model(rid));
                let is_sel = selected.as_deref() == Some(rid.as_str());
                col = col.child(card::view(model, is_sel));
            }
        }

        let rest: Vec<HfModel> = models
            .into_iter()
            .filter(|m| !pinned.iter().any(|p| p == &m.id))
            .collect();
        if !pinned.is_empty() && !rest.is_empty() {
            col = col.child(Text::new(tr!("hf.list.models_section")).class("hf-list-section"));
        }
        for m in rest.into_iter() {
            let is_sel = selected.as_deref() == Some(m.id.as_str());
            col = col.child(card::view(m, is_sel));
        }

        let scroll = ScrollView::new().vertical().class("hf-list-scroll").child(col);
        vec![Box::new(scroll)]
    })
}

/// Минимальная карточка-модель для закреплённого качающегося репозитория,
/// которого нет в текущем списке поиска (id известен из `downloads`).
fn minimal_model(repo_id: &str) -> HfModel {
    HfModel {
        id: repo_id.to_string(),
        author: None,
        downloads: 0,
        likes: 0,
        last_modified: None,
        tags: Vec::new(),
        pipeline_tag: None,
        library_name: None,
        private: false,
        disabled: false,
    }
}

fn loading_skeleton() -> impl Widget {
    let mut col = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch);
    for _ in 0..6 {
        col = col.child(DecoratedBox::new().class("hf-card loading"));
    }
    ScrollView::new().vertical().class("hf-list-scroll").child(col)
}

fn error_view() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let msg = ctx
            .list_error
            .get()
            .unwrap_or_else(|| tr!("hf.list.unknown_error"));
        let widget = mgui! {
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::Center)
                .class("hf-list-error") => [
                    Text::new(tr!("hf.list.load_error_title"))
                        .class("hf-list-error-title"),
                    Text::new(msg).class("hf-list-error-msg"),
                    Button::new(tr!("hf.list.retry"))
                        .on_click(actions::trigger_list_reload)
                        .class("hf-retry-btn"),
                ]
        };
        vec![Box::new(Center::new().child(widget))]
    })
}

fn empty_view() -> impl Widget {
    Center::new().child(
        Text::new(tr!("hf.list.empty")).class("hf-list-empty"),
    )
}
