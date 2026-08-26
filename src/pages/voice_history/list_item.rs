//! Один элемент списка истории голосовых записей.
//!
//! Карточка с превью текста, длительностью, датой. Клик → выбрать запись.
//! Иконка-корзина → удалить (с подтверждением через snackbar в будущем,
//! пока — прямое удаление; запись восстановить нельзя).

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::GestureDetector;

use crate::context::AppCtx;
use crate::icons::MI_DELETE;

use super::storage::VoiceRecording;

pub fn view(rec: VoiceRecording, is_active: bool) -> impl Widget {
    let class = if is_active {
        "voice-history-item voice-history-item-selected"
    } else {
        "voice-history-item"
    };

    let preview = preview_text(&rec.transcript);
    let duration = format_duration(rec.duration_ms);
    let date = format_date(rec.created_at);

    let id_for_select = rec.id.clone();
    let id_for_delete = rec.id.clone();

    mgui! {
        DecoratedBox::new().class(class) => [
            GestureDetector::new()
                .on_click(move || {
                    let app = use_context::<AppCtx>();
                    app.voice_history.selected.set(Some(id_for_select.clone()));
                })
                .child(mgui! {
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center) => [
                            DecoratedBox::new().class("grow") => [
                                Column::new()
                                    .gap(4.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                                        Text::new(preview).class("voice-history-item-preview"),
                                        Row::new()
                                            .gap(8.0)
                                            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                                                Text::new(duration).class("voice-history-item-meta"),
                                                Text::new("·").class("voice-history-item-meta-sep"),
                                                Text::new(date).class("voice-history-item-meta")
                                            ]
                                    ]
                            ],
                            ToolButton::new(MI_DELETE)
                                .tooltip(tr!("voice.history.item.delete_tooltip"))
                                .on_click(move || {
                                    let app = use_context::<AppCtx>();
                                    let id = id_for_delete.clone();
                                    super::storage::delete_recording(&id);
                                    app.voice_history.recordings.update(|v| v.retain(|r| r.id != id));
                                    if app.voice_history.selected.get_untracked().as_deref() == Some(id.as_str()) {
                                        app.voice_history.selected.set(None);
                                    }
                                })
                                .class("voice-history-item-delete")
                        ]
                })
        ]
    }
}

fn preview_text(s: &str) -> String {
    let trimmed = s.trim();
    let max_chars = 80;
    let mut out = String::new();
    let mut count = 0;
    for c in trimmed.chars() {
        if count >= max_chars {
            out.push('…');
            break;
        }
        out.push(c);
        count += 1;
    }
    if out.is_empty() {
        tr!("voice.history.item.empty_preview")
    } else {
        out
    }
}

fn format_duration(ms: u32) -> String {
    let total_secs = ms / 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    if mins > 0 {
        tr!(
            "voice.history.item.duration.min_sec",
            min = mins.to_string(),
            sec = format!("{secs:02}")
        )
    } else {
        tr!("voice.history.item.duration.sec", sec = secs.to_string())
    }
}

/// «Сегодня 14:23» / «Вчера 09:11» / «N дней назад HH:MM» в зависимости от
/// давности записи. Без chrono — арифметика по unix-секундам в UTC + смещение
/// текущей timezone берём из `time::OffsetDateTime::now_local()` если получится,
/// иначе UTC.
fn format_date(unix_secs: u64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if unix_secs == 0 || unix_secs > now {
        return String::new();
    }

    let secs_per_day = 86_400;
    let now_day = now / secs_per_day;
    let rec_day = unix_secs / secs_per_day;
    let days_ago = now_day.saturating_sub(rec_day);

    // Час и минуты в UTC. Для MVP не делаем локальный TZ-offset — это требует
    // tz-resolver'а; UTC-метка лучше пустой. Можно дописать в Sprint 3.
    let secs_in_day = unix_secs % secs_per_day;
    let h = (secs_in_day / 3600) as u32;
    let m = ((secs_in_day % 3600) / 60) as u32;

    let time = format!("{h:02}:{m:02}");
    if days_ago == 0 {
        tr!("voice.history.item.date.today", time = time)
    } else if days_ago == 1 {
        tr!("voice.history.item.date.yesterday", time = time)
    } else if days_ago < 7 {
        tr!(
            "voice.history.item.date.days_ago_time",
            days = days_ago.to_string(),
            time = time
        )
    } else {
        tr!("voice.history.item.date.days_ago", days = days_ago.to_string())
    }
}
