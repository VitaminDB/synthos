//! Элементы поиска по Hub: поле запроса (встаёт в общую шапку рядом с
//! пилюлей глобального поиска) и сортировочные чипы (над списком моделей).

use syngui::prelude::*;
use syngui::widget::styled::WidgetExt;
use syngui::widgets::TextField;

use crate::icons::MI_SEARCH;

use super::actions;
use super::state::{HuggingFaceCtx, SortMode};

/// Поле поиска по Hub для центра общей шапки.
pub fn search_bar() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let current = ctx.search_query.get_untracked();
        // TextField сам — DecoratedBox с padding/border/radius из MSS. Никаких
        // дополнительных обёрток: они дают двойной бордер.
        let field = TextField::new()
            .text(current)
            .placeholder(tr!("hf.header.search_placeholder"))
            .prefix_icon(MI_SEARCH)
            .on_change(move |s| {
                let ctx = use_context::<HuggingFaceCtx>();
                ctx.search_query.set(s.to_string());
            })
            .on_submit(move |_| actions::commit_search())
            .class("hf-search-bar");
        vec![Box::new(field)]
    })
}

/// Чипы сортировки + фильтр GGUF — над списком моделей.
pub fn sort_chips() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let current = ctx.sort_mode.get();
        let mut row = Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center);
        for mode in [
            SortMode::Trending,
            SortMode::MostDownloads,
            SortMode::MostLikes,
            SortMode::RecentlyUpdated,
        ] {
            row = row.child(chip(mode, current == mode));
        }
        if ctx.gguf_support.get() {
            row = row.child(gguf_chip(ctx.gguf_filter.get()));
        }
        vec![Box::new(row)]
    })
}

fn gguf_chip(selected: bool) -> impl Widget {
    let class = if selected { "hf-chip selected" } else { "hf-chip" };
    Button::new("GGUF")
        .on_click(move || {
            let ctx = use_context::<HuggingFaceCtx>();
            let now = !ctx.gguf_filter.get_untracked();
            ctx.gguf_filter.set(now);
            actions::trigger_list_reload();
        })
        .class(class)
}

fn chip(mode: SortMode, selected: bool) -> impl Widget {
    let class = if selected { "hf-chip selected" } else { "hf-chip" };
    Button::new(mode.label())
        .on_click(move || {
            let ctx = use_context::<HuggingFaceCtx>();
            if ctx.sort_mode.get_untracked() != mode {
                ctx.sort_mode.set(mode);
                actions::trigger_list_reload();
            }
        })
        .class(class)
}
