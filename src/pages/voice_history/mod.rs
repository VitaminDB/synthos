//! Страница «История голоса» — список сохранённых распознаваний и плеер.
//!
//! Структура: `Row [ list (360px) | player_pane (grow) ]`. Список реактивно
//! читает `app.voice_history.recordings`, выбор записи кладёт id в
//! `voice_history.selected`. Правая колонка реактивно показывает плеер
//! выбранной записи или placeholder «выберите запись».

pub mod list_item;
pub mod player_pane;
pub mod storage;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::Center;

use crate::context::AppCtx;

pub fn view() -> impl Widget {
    mgui! {
        DecoratedBox::new().class("voice-history-page") => [
            Row::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    DecoratedBox::new().class("voice-history-list-pane").child(list_view()),
                    DecoratedBox::new().class("voice-history-detail-pane").child(detail_view())
                ]
        ]
    }
}

/// Реактивный список — пересобирается при изменении recordings.
fn list_view() -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        let recordings = app.voice_history.recordings.get();
        let selected = app.voice_history.selected.get();

        if recordings.is_empty() {
            return DecoratedBox::new()
                .class("voice-history-list voice-history-list-empty")
                .child(Center::new().child(
                    Text::new(tr!("voice.history.list.empty")).class("voice-history-empty-text"),
                ));
        }

        let mut col = Column::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch);
        for rec in &recordings {
            let is_active = selected.as_deref() == Some(rec.id.as_str());
            col = col.child(list_item::view(rec.clone(), is_active));
        }
        DecoratedBox::new().class("voice-history-list").child(col)
    }
}

/// Реактивная карточка плеера — реагирует на смену `selected`.
fn detail_view() -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        let selected_id = app.voice_history.selected.get();
        let _ = app.voice_history.recordings.get(); // подписка для refresh после delete

        let Some(id) = selected_id else {
            return DecoratedBox::new().class("voice-history-detail-empty").child(
                Center::new().child(
                    Text::new(tr!("voice.history.detail.select_hint")).class("voice-history-empty-text"),
                ),
            );
        };

        let recordings = app.voice_history.recordings.get_untracked();
        let Some(rec) = recordings.iter().find(|r| r.id == id).cloned() else {
            return DecoratedBox::new().class("voice-history-detail-empty").child(
                Center::new().child(
                    Text::new(tr!("voice.history.detail.not_found")).class("voice-history-empty-text"),
                ),
            );
        };

        DecoratedBox::new()
            .class("voice-history-detail")
            .child(player_pane::view(rec))
    }
}
