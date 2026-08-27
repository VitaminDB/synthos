//! Ранжирование запроса по элементам индекса.
//!
//! Одна строка запроса разбивается на токены, каждый обязан найтись хоть в
//! одном поле элемента: заголовок, подзаголовок, подсказка или ключевое
//! слово. Точное совпадение весит больше префикса, префикс — больше начала
//! слова, дальше — вхождение и, в самом конце, компактная подпоследовательность
//! («тлв» → «тепловычислитель»). Дополнительно запрос прогоняется через
//! смену раскладки (`ghb,jh` → `прибор`) — со штрафом, чтобы прямое
//! совпадение всегда было выше.

use super::model::SearchItem;

pub const SCORE_EXACT: i32 = 100;
pub const SCORE_PREFIX: i32 = 90;
pub const SCORE_WORD: i32 = 80;
pub const SCORE_CONTAINS: i32 = 60;
pub const SCORE_SUBSEQUENCE: i32 = 30;
pub const TITLE_BONUS: i32 = 12;
pub const SUBTITLE_BONUS: i32 = 4;
pub const LAYOUT_PENALTY: i32 = 8;
const SUBSEQUENCE_MIN_CHARS: usize = 3;
const SUBSEQUENCE_MAX_SPREAD_FACTOR: usize = 3;
const SUBSEQUENCE_MAX_SPREAD_EXTRA: usize = 4;
const TITLE_LENGTH_PENALTY_STEP: usize = 24;

const LAYOUT_EN: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,./`";
const LAYOUT_RU: &str = "йцукенгшщзхъфывапролджэячсмитьбю.ё";

/// Посимвольная нормализация: нижний регистр, «ё» → «е». Число символов
/// сохраняется, поэтому индексы совпадений переносятся на исходную строку.
pub fn normalize(s: &str) -> String {
    s.chars().map(fold_char).collect()
}

fn fold_char(c: char) -> char {
    let c = c.to_lowercase().next().unwrap_or(c);
    match c {
        'ё' => 'е',
        _ => c,
    }
}

/// Текст, набранный не в той раскладке: латиница переводится в кириллицу по
/// позициям клавиш ЙЦУКЕН и наоборот. `None`, если раскладки смешаны или
/// переводить нечего.
pub fn switch_layout(s: &str) -> Option<String> {
    let latin = s.chars().filter(|c| c.is_ascii_alphabetic()).count();
    let cyrillic = s
        .chars()
        .filter(|c| ('а'..='я').contains(c) || *c == 'ё')
        .count();
    let (from, to) = match (latin > 0, cyrillic > 0) {
        (true, false) => (LAYOUT_EN, LAYOUT_RU),
        (false, true) => (LAYOUT_RU, LAYOUT_EN),
        _ => return None,
    };
    let from: Vec<char> = from.chars().collect();
    let to: Vec<char> = to.chars().collect();
    let switched: String = s
        .chars()
        .map(|c| match from.iter().position(|f| *f == c) {
            Some(i) => to[i],
            None => c,
        })
        .collect();
    let has_letters = switched.chars().any(|c| c.is_alphabetic());
    if switched == s || !has_letters {
        return None;
    }
    Some(switched)
}

pub struct QueryPlan {
    pub tokens: Vec<String>,
    pub switched: Option<(String, Vec<String>)>,
}

pub fn plan(query: &str) -> Option<QueryPlan> {
    let norm = normalize(query);
    let tokens: Vec<String> = norm.split_whitespace().map(String::from).collect();
    if tokens.is_empty() {
        return None;
    }
    let switched = switch_layout(norm.trim()).map(|alt| {
        let alt_tokens = alt.split_whitespace().map(String::from).collect();
        (alt, alt_tokens)
    });
    Some(QueryPlan { tokens, switched })
}

pub struct FieldMatch {
    pub score: i32,
    pub ranges: Vec<(usize, usize)>,
}

/// Совпадение токена с нормализованным полем; диапазоны — в символах.
pub fn match_field(field: &str, token: &str) -> Option<FieldMatch> {
    if field.is_empty() || token.is_empty() {
        return None;
    }
    let token_len = token.chars().count();
    if field == token {
        return Some(FieldMatch {
            score: SCORE_EXACT,
            ranges: vec![(0, token_len)],
        });
    }
    if field.starts_with(token) {
        return Some(FieldMatch {
            score: SCORE_PREFIX,
            ranges: vec![(0, token_len)],
        });
    }
    let field_chars: Vec<char> = field.chars().collect();
    let token_chars: Vec<char> = token.chars().collect();
    if let Some(pos) = find_word_start(&field_chars, &token_chars) {
        return Some(FieldMatch {
            score: SCORE_WORD,
            ranges: vec![(pos, pos + token_len)],
        });
    }
    if let Some(pos) = find_chars(&field_chars, &token_chars) {
        return Some(FieldMatch {
            score: SCORE_CONTAINS,
            ranges: vec![(pos, pos + token_len)],
        });
    }
    if token_len >= SUBSEQUENCE_MIN_CHARS {
        return match_subsequence(&field_chars, &token_chars);
    }
    None
}

fn find_chars(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn find_word_start(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .enumerate()
        .position(|(i, w)| w == needle && (i == 0 || !haystack[i - 1].is_alphanumeric()))
}

fn match_subsequence(haystack: &[char], needle: &[char]) -> Option<FieldMatch> {
    let mut positions = Vec::with_capacity(needle.len());
    let mut want = needle.iter();
    let mut next = want.next();
    for (i, c) in haystack.iter().enumerate() {
        if Some(c) == next {
            positions.push(i);
            next = want.next();
            if next.is_none() {
                break;
            }
        }
    }
    if next.is_some() {
        return None;
    }
    let first = *positions.first()?;
    let last = *positions.last()?;
    let spread = last - first + 1;
    if spread > needle.len() * SUBSEQUENCE_MAX_SPREAD_FACTOR + SUBSEQUENCE_MAX_SPREAD_EXTRA {
        return None;
    }
    let looseness = (spread - needle.len()) as i32;
    Some(FieldMatch {
        score: SCORE_SUBSEQUENCE - looseness.min(20),
        ranges: merge_ranges(positions.iter().map(|p| (*p, *p + 1)).collect()),
    })
}

pub fn merge_ranges(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

#[derive(Clone, Debug)]
pub struct ItemMatch {
    pub score: i32,
    pub title_ranges: Vec<(usize, usize)>,
    pub matched: Option<String>,
    pub via_layout: bool,
}

pub fn score_item(item: &SearchItem, plan: &QueryPlan) -> Option<ItemMatch> {
    let primary = score_tokens(item, &plan.tokens, false);
    let switched = plan
        .switched
        .as_ref()
        .and_then(|(_, tokens)| score_tokens(item, tokens, true));
    match (primary, switched) {
        (Some(p), Some(s)) => Some(if s.score > p.score { s } else { p }),
        (Some(p), None) => Some(p),
        (None, Some(s)) => Some(s),
        (None, None) => None,
    }
}

fn score_tokens(item: &SearchItem, tokens: &[String], via_layout: bool) -> Option<ItemMatch> {
    let mut total = 0;
    let mut ranges = Vec::new();
    let mut matched: Option<String> = None;
    for token in tokens {
        let mut best_score = i32::MIN;
        let mut best_ranges: Option<Vec<(usize, usize)>> = None;
        let mut best_field: Option<&str> = None;
        if let Some(m) = match_field(&item.norm_title, token) {
            best_score = m.score + TITLE_BONUS;
            best_ranges = Some(m.ranges);
        }
        for field in &item.fields {
            if let Some(m) = match_field(&field.norm, token) {
                let score = m.score + field.bonus;
                if score > best_score {
                    best_score = score;
                    best_ranges = None;
                    best_field = Some(&field.display);
                }
            }
        }
        if best_score == i32::MIN {
            return None;
        }
        total += best_score;
        match best_ranges {
            Some(r) => ranges.extend(r),
            None => {
                if matched.is_none() {
                    matched = best_field.map(str::to_string);
                }
            }
        }
    }
    if via_layout {
        total -= LAYOUT_PENALTY;
    }
    total -= (item.title.chars().count() / TITLE_LENGTH_PENALTY_STEP) as i32;
    Some(ItemMatch {
        score: total,
        title_ranges: merge_ranges(ranges),
        matched,
        via_layout,
    })
}

/// Разбивает текст на отрезки (текст, подсвечен) по диапазонам в символах.
pub fn split_highlight(text: &str, ranges: &[(usize, usize)]) -> Vec<(String, bool)> {
    let chars: Vec<char> = text.chars().collect();
    let mut segments: Vec<(String, bool)> = Vec::new();
    let mut cursor = 0;
    for &(start, end) in ranges {
        let start = start.min(chars.len());
        let end = end.min(chars.len());
        if start < cursor || start >= end {
            continue;
        }
        if start > cursor {
            segments.push((chars[cursor..start].iter().collect(), false));
        }
        segments.push((chars[start..end].iter().collect(), true));
        cursor = end;
    }
    if cursor < chars.len() {
        segments.push((chars[cursor..].iter().collect(), false));
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::model::{SearchAction, SearchKind};

    fn item(title: &str) -> SearchItem {
        SearchItem::new("k", SearchKind::Chat, title, SearchAction::Chat("k".into()))
    }

    #[test]
    fn normalizes_case_and_yo() {
        assert_eq!(normalize("Тёплый Дом"), "теплый дом");
        assert_eq!(normalize("ABC").chars().count(), 3);
    }

    #[test]
    fn switches_keyboard_layout_both_ways() {
        assert_eq!(switch_layout("yjls").as_deref(), Some("ноды"));
        assert_eq!(switch_layout("xfn").as_deref(), Some("чат"));
        assert_eq!(switch_layout("срфе").as_deref(), Some("chat"));
        assert_eq!(switch_layout("123"), None);
        assert_eq!(switch_layout("abc абв"), None);
    }

    #[test]
    fn ranks_prefix_over_word_over_contains_over_subsequence() {
        let prefix = match_field("настройки темы", "настрой").unwrap();
        let word = match_field("редактор кода", "код").unwrap();
        let contains = match_field("компактификация", "актиф").unwrap();
        let subseq = match_field("редактор", "рдкт").unwrap();
        assert!(prefix.score > word.score);
        assert!(word.score > contains.score);
        assert!(contains.score > subseq.score);
        assert_eq!(word.ranges, vec![(9, 12)]);
        assert!(match_field("настройки", "xyz").is_none());
    }

    #[test]
    fn subsequence_requires_compactness() {
        assert!(match_field("абвгдежзийклмнопрстуфхцчшщ", "ащ").is_none());
        assert!(
            match_field("а б в г д е ж з и й к л м н о п р с т у ф х ц ч ш щ", "ацш").is_none()
        );
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

    #[test]
    fn wrong_layout_still_finds_but_ranks_lower() {
        let plan = plan("yjls").unwrap();
        let it = item("Ноды");
        let m = score_item(&it, &plan).unwrap();
        assert!(m.via_layout);
        let direct = score_item(&it, &super::plan("ноды").unwrap()).unwrap();
        assert!(direct.score > m.score);
    }

    #[test]
    fn highlight_segments_cover_whole_text() {
        let segs = split_highlight("Новый чат", &[(0, 5)]);
        assert_eq!(
            segs,
            vec![("Новый".to_string(), true), (" чат".to_string(), false)]
        );
        let joined: String = split_highlight("abcdef", &[(1, 2), (4, 5)])
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        assert_eq!(joined, "abcdef");
    }

    #[test]
    fn merges_overlapping_ranges() {
        assert_eq!(
            merge_ranges(vec![(3, 5), (0, 2), (4, 8), (2, 3)]),
            vec![(0, 8)]
        );
        assert_eq!(merge_ranges(vec![(5, 6), (0, 2)]), vec![(0, 2), (5, 6)]);
    }
}
