//! Верхняя часть страницы HuggingFace: заголовок + warning + search-bar +
//! сортировочные чипы.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::WidgetExt;
use syngui::widgets::TextField;

use crate::icons::MI_SEARCH;

use super::actions;
use super::state::{HuggingFaceCtx, SortMode};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("hf-header").child(mgui! {
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                title_row(),
                search_bar(),
                sort_chips(),
            ]
    })
}

fn title_row() -> impl Widget {
    mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new("HuggingFace").class("hf-title"),
                Text::new("поиск моделей по hub'у").class("hf-subtitle"),
            ]
    }
}

fn search_bar() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<HuggingFaceCtx>();
        let current = ctx.search_query.get_untracked();
        // TextField сам — DecoratedBox с padding/border/radius из MSS. Никаких
        // дополнительных обёрток: они дают двойной бордер.
        let field = TextField::new()
            .text(current)
            .placeholder("Поиск моделей…")
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

fn sort_chips() -> impl Widget {
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
        vec![Box::new(row)]
    })
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
