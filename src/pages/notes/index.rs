//! Индекс wiki-связей vault'а.
//!
//! Лёгкий лексер по сырому тексту `.md` собирает `[[цели]]` (и `![[врезки]]`
//! — они тоже связи), минуя fenced-код. Цель резолвится по имени страницы
//! (без расширения, без регистра) либо по vault-относительному пути
//! (`Папка/Имя`). Полный скан дешёвый (килобайты текста), инкрементальное
//! обновление одной страницы — на каждое сохранение.

use std::collections::HashMap;
use std::path::Path;

use super::storage;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VaultIndex {
    /// title в нижнем регистре → rel-путь `.md`.
    by_title: HashMap<String, String>,
    /// rel-путь без расширения в нижнем регистре → rel-путь.
    by_path: HashMap<String, String>,
    /// rel → сырые цели исходящих ссылок.
    outgoing: HashMap<String, Vec<String>>,
    /// rel → страницы, ссылающиеся на неё.
    backlinks: HashMap<String, Vec<String>>,
}

impl VaultIndex {
    /// Полный скан vault'а.
    pub fn build(root: &Path) -> Self {
        let mut idx = Self::default();
        for entry in storage::scan(root) {
            if entry.kind != storage::VaultEntryKind::Dir {
                idx.register_page(&entry.rel);
            }
            if entry.kind == storage::VaultEntryKind::Page {
                if let Ok(content) = storage::load(root, &entry.rel) {
                    idx.outgoing.insert(entry.rel.clone(), lex_links(&content));
                }
            }
        }
        idx.rebuild_backlinks();
        idx
    }

    fn register_page(&mut self, rel: &str) {
        self.by_title.insert(storage::title_of(rel).to_lowercase(), rel.to_string());
        let no_ext = strip_known_ext(rel).to_lowercase();
        self.by_path.insert(no_ext, rel.to_string());
    }

    /// Обновление одной страницы после сохранения.
    pub fn update_page(&mut self, rel: &str, content: &str) {
        self.register_page(rel);
        self.outgoing.insert(rel.to_string(), lex_links(content));
        self.rebuild_backlinks();
    }

    fn rebuild_backlinks(&mut self) {
        self.backlinks.clear();
        let pairs: Vec<(String, String)> = self
            .outgoing
            .iter()
            .flat_map(|(from, targets)| {
                targets
                    .iter()
                    .filter_map(|t| self.resolve(t))
                    .map(|to| (to, from.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (to, from) in pairs {
            let list = self.backlinks.entry(to).or_default();
            if !list.contains(&from) {
                list.push(from);
            }
        }
        for list in self.backlinks.values_mut() {
            list.sort();
        }
    }

    /// Цель ссылки → rel-путь существующей страницы.
    pub fn resolve(&self, target: &str) -> Option<String> {
        let key = target.trim().to_lowercase();
        self.by_title
            .get(&key)
            .or_else(|| self.by_path.get(&key))
            .cloned()
    }

    /// Кандидаты автокомплита по префиксу/подстроке.
    pub fn complete(&self, prefix: &str) -> Vec<(String, String)> {
        let q = prefix.trim().to_lowercase();
        let mut out: Vec<(String, String)> = self
            .by_title
            .iter()
            .filter(|(title, _)| q.is_empty() || title.contains(&q))
            .map(|(_, rel)| (storage::title_of(rel), rel.clone()))
            .collect();
        out.sort_by(|a, b| {
            // Совпадение с начала — выше.
            let a_starts = a.0.to_lowercase().starts_with(&q);
            let b_starts = b.0.to_lowercase().starts_with(&q);
            b_starts.cmp(&a_starts).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
        });
        out.truncate(12);
        out
    }

    /// Кто ссылается на страницу.
    pub fn backlinks_of(&self, rel: &str) -> Vec<String> {
        self.backlinks.get(rel).cloned().unwrap_or_default()
    }

    /// Исходящие цели страницы (разрезолвленные).
    pub fn outgoing_of(&self, rel: &str) -> Vec<String> {
        self.outgoing
            .get(rel)
            .map(|targets| targets.iter().filter_map(|t| self.resolve(t)).collect())
            .unwrap_or_default()
    }

    /// Все страницы с исходящими связями — для графа (T10).
    pub fn pages(&self) -> Vec<String> {
        let mut v: Vec<String> = self.by_title.values().cloned().collect();
        v.sort();
        v.dedup();
        v
    }
}

fn strip_known_ext(rel: &str) -> &str {
    for suffix in [".base.json", ".canvas.json", ".md"] {
        if rel.len() < suffix.len() {
            continue;
        }
        let idx = rel.len() - suffix.len();
        // Не-ASCII имя: байтовый индекс может попасть внутрь многобайтового
        // символа — такой хвост суффиксом быть не может.
        if rel.is_char_boundary(idx) && rel[idx..].eq_ignore_ascii_case(suffix) {
            return &rel[..idx];
        }
    }
    rel
}

/// Сырые цели `[[...]]` в md-тексте; fenced-код пропускается.
pub fn lex_links(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut rest = line;
        while let Some(open) = rest.find("[[") {
            let after = &rest[open + 2..];
            let Some(close) = after.find("]]") else { break };
            let inner = &after[..close];
            if !inner.is_empty() && !inner.contains('[') && !inner.contains(']') {
                let target = inner.split('|').next().unwrap_or(inner).trim();
                if !target.is_empty() {
                    out.push(target.to_string());
                }
            }
            rest = &after[close + 2..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexer_finds_links_and_skips_code() {
        let md = "текст [[Раз]] и ![[Два|врезка]]\n```\n[[не ссылка]]\n```\n[[Три]]";
        assert_eq!(lex_links(md), vec!["Раз", "Два", "Три"]);
    }

    #[test]
    fn strip_known_ext_cyrillic_no_panic() {
        // Байтовый индекс len-10 (".base.json") попадает внутрь кириллицы —
        // раньше здесь была паника "not a char boundary".
        assert_eq!(strip_known_ext("Новая заметка.md"), "Новая заметка");
        assert_eq!(strip_known_ext("Заметки.canvas.json"), "Заметки");
        assert_eq!(strip_known_ext("Без расширения"), "Без расширения");
    }
}
