//! Ранжирование запроса по элементам индекса. Движок — общий
//! [`syngui::fuzzy`]; здесь — привязка к [`SearchItem`].

pub use syngui::fuzzy::{
    merge_ranges, normalize, plan, split_highlight, switch_layout, ItemMatch, QueryPlan, SearchField,
    SUBTITLE_BONUS, TITLE_BONUS,
};

use super::model::SearchItem;

pub fn score_item(item: &SearchItem, plan: &QueryPlan) -> Option<ItemMatch> {
    syngui::fuzzy::score(&item.title, &item.norm_title, &item.fields, plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::model::{SearchAction, SearchKind};

    fn item(title: &str) -> SearchItem {
        SearchItem::new("k", SearchKind::Chat, title, SearchAction::Chat("k".into()))
    }

    #[test]
    fn every_token_must_match_and_title_gets_bonus() {
        let plan = plan("модели контекст").unwrap();
        let hit = item("Загрузка модели").subtitle("контекст 128k");
        let miss = item("Загрузка модели").subtitle("qwen3.8-27b");
        assert!(score_item(&hit, &plan).is_some());
        assert!(score_item(&miss, &plan).is_none());
    }

    #[test]
    fn keyword_match_reports_matched_field() {
        let plan = plan("ffmpeg").unwrap();
        let it = item("Новый чат").keyword("ffmpeg");
        let m = score_item(&it, &plan).unwrap();
        assert_eq!(m.matched.as_deref(), Some("ffmpeg"));
        assert!(m.title_ranges.is_empty());
    }
}
