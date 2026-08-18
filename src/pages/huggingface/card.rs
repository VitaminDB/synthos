//! Карточка одной модели в списке слева. Кликабельна через `GestureDetector`
//! (DecoratedBox сам по себе click'и не ловит).

use syngui::mgui;
use syngui::mss::{MssColor, StyleValue};
use syngui::prelude::*;
use syngui::widget::styled::WidgetExt;
use syngui::widgets::containers::GestureDetector;

use crate::icons::{MI_DOWNLOAD, MI_FAVORITE, MI_AUTO_AWESOME};

use super::actions;
use super::state::HfModel;

pub fn view(model: HfModel, is_selected: bool) -> impl Widget {
    let class = if is_selected { "hf-card selected" } else { "hf-card" };
    let repo_id = model.id.clone();
    let author_line = model.author.clone().unwrap_or_else(|| {
        model
            .id
            .split('/')
            .next()
            .map(String::from)
            .unwrap_or_else(|| "—".into())
    });
    let pipeline = model
        .pipeline_tag
        .clone()
        .unwrap_or_else(|| model.library_name.clone().unwrap_or_default());
    let dl_text = human_count(model.downloads);
    let likes_text = human_count(model.likes);
    let modified_text = short_date(model.last_modified.as_deref());

    let author_for_avatar = model.author.clone().unwrap_or_else(|| {
        model
            .id
            .split('/')
            .next()
            .map(String::from)
            .unwrap_or_default()
    });
    let body = DecoratedBox::new().class(class).child(mgui! {
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Start) => [
                avatar_widget(&author_for_avatar),
                Column::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start) => [
                        Text::new(model.id.clone()).class("hf-card-title"),
                        Text::new(author_line).class("hf-card-author"),
                        stats_row(dl_text, likes_text, modified_text),
                        pipeline_row(pipeline),
                    ],
            ]
    });

    GestureDetector::new()
        .child(body)
        .on_click(move || actions::select_model(repo_id.clone()))
}

/// Letter-avatar: круглый бейдж с первой буквой автора и детерминированным
/// цветом из хэша имени. Без сети — HF не отдаёт avatar через прямой URL
/// без авторизации (`/{author}/avatar` → 401). Если автор пустой, рисуем
/// иконку-фолбэк. Возвращается Reactive, чтобы оба варианта (Icon vs
/// DecoratedBox) имели единый widget-тип.
fn avatar_widget(author: &str) -> impl Widget {
    let author = author.to_string();
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if author.is_empty() {
            return vec![Box::new(Icon::new(MI_AUTO_AWESOME).class("hf-card-avatar"))];
        }
        let (letter, bg) = letter_and_color(&author);
        let badge = DecoratedBox::new()
            .class("hf-card-avatar-letter")
            .style("background-color", StyleValue::Color(bg))
            .child(Center::new().child(
                Text::new(letter).class("hf-card-avatar-letter-text"),
            ));
        vec![Box::new(badge)]
    })
}

/// Возвращает (первая буква в верхнем регистре, deterministic-цвет из палитры).
fn letter_and_color(name: &str) -> (String, MssColor) {
    let first = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().next().unwrap_or(c))
        .unwrap_or('?');
    // 12 акцентных цветов — читаемые на белой букве, не сливаются между
    // соседними карточками.
    const PALETTE: &[MssColor] = &[
        MssColor::rgb(0x3B, 0x82, 0xF6),
        MssColor::rgb(0x10, 0xB9, 0x81),
        MssColor::rgb(0xF5, 0x9E, 0x0B),
        MssColor::rgb(0xEF, 0x44, 0x44),
        MssColor::rgb(0x8B, 0x5C, 0xF6),
        MssColor::rgb(0xEC, 0x48, 0x99),
        MssColor::rgb(0x14, 0xB8, 0xA6),
        MssColor::rgb(0xF9, 0x73, 0x16),
        MssColor::rgb(0x84, 0xCC, 0x16),
        MssColor::rgb(0x06, 0xB6, 0xD4),
        MssColor::rgb(0xA8, 0x55, 0xF7),
        MssColor::rgb(0x64, 0x74, 0x8B),
    ];
    let h: u32 = name.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let bg = PALETTE[(h as usize) % PALETTE.len()];
    (first.to_string(), bg)
}

fn stats_row(dl: String, likes: String, modified: String) -> impl Widget {
    mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("hf-card-stats") => [
                Icon::new(MI_DOWNLOAD).class("hf-stat-icon"),
                Text::new(dl).class("hf-stat-text"),
                Icon::new(MI_FAVORITE).class("hf-stat-icon"),
                Text::new(likes).class("hf-stat-text"),
                Text::new(modified).class("hf-stat-time"),
            ]
    }
}

fn pipeline_row(pipeline: String) -> impl Widget {
    let class = if pipeline.is_empty() {
        "hf-card-pipeline-empty"
    } else {
        "hf-card-pipeline-chip"
    };
    let text = if pipeline.is_empty() {
        String::new()
    } else {
        pipeline
    };
    DecoratedBox::new()
        .class(class)
        .child(Text::new(text).class("hf-card-pipeline-text"))
}

/// «1.2M» / «830K» / «42» — компактное отображение больших чисел.
fn human_count(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// ISO `2026-05-08T...` → `2026-05-08`. Если строка не похожа на ISO,
/// возвращаем её целиком (UI покажет как есть).
fn short_date(iso: Option<&str>) -> String {
    let Some(s) = iso else { return String::new() };
    if s.len() < 10 {
        return s.to_string();
    }
    s.chars().take(10).collect()
}
