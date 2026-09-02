//! Индекс wiki-связей проекта.
//!
//! Лёгкий лексер по сырому markdown собирает `[[цели]]` (и `![[врезки]]`
//! — они тоже связи), минуя fenced-код. Цель — название страницы (без
//! регистра); ссылки на объекты (`kanban:<id>`, `gantt:<id>`) не считаются
//! связями между страницами. Ключ всюду — id страницы из дерева проекта.

use std::collections::HashMap;

use super::project::ProjectTree;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VaultIndex {
    /// id → название.
    titles: HashMap<String, String>,
    /// название в нижнем регистре → id (первая страница с таким именем).
    by_title: HashMap<String, String>,
    /// id → сырые цели исходящих ссылок.
    outgoing: HashMap<String, Vec<String>>,
    /// id → страницы, ссылающиеся на неё.
    backlinks: HashMap<String, Vec<String>>,
}

impl VaultIndex {
    /// Полный скан: названия из дерева, ссылки из содержимого страниц.
    pub fn build(tree: &ProjectTree, content_of: impl Fn(&str) -> Option<String>) -> Self {
        let mut idx = Self::default();
        idx.set_titles(tree);
        for node in tree.all() {
            if let Some(content) = content_of(&node.id) {
                idx.outgoing.insert(node.id.clone(), lex_links(&content));
            }
        }
        idx.rebuild_backlinks();
        idx
    }

    /// Пересобрать названия после правки дерева (переименование, удаление,
    /// создание); ссылки удалённых страниц выбрасываются.
    pub fn set_titles(&mut self, tree: &ProjectTree) {
        self.titles.clear();
        self.by_title.clear();
        for node in tree.all() {
            self.titles.insert(node.id.clone(), node.title.clone());
            self.by_title
                .entry(node.title.trim().to_lowercase())
                .or_insert_with(|| node.id.clone());
        }
        self.outgoing.retain(|id, _| self.titles.contains_key(id));
        self.rebuild_backlinks();
    }

    /// Обновление одной страницы после сохранения.
    pub fn update_page(&mut self, id: &str, content: &str) {
        self.outgoing.insert(id.to_string(), lex_links(content));
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
            if to == from {
                continue;
            }
            let list = self.backlinks.entry(to).or_default();
            if !list.contains(&from) {
                list.push(from);
            }
        }
        for list in self.backlinks.values_mut() {
            list.sort();
        }
    }

    /// Цель ссылки → id существующей страницы. Принимает и `id:<id>`.
    pub fn resolve(&self, target: &str) -> Option<String> {
        let t = target.trim();
        if let Some(id) = t.strip_prefix("id:") {
            return self.titles.contains_key(id).then(|| id.to_string());
        }
        self.by_title.get(&t.to_lowercase()).cloned()
    }

    pub fn title_of(&self, id: &str) -> String {
        self.titles.get(id).cloned().unwrap_or_else(|| id.to_string())
    }

    /// Кандидаты автокомплита: (название, id).
    pub fn complete(&self, prefix: &str) -> Vec<(String, String)> {
        let q = prefix.trim().to_lowercase();
        let mut out: Vec<(String, String)> = self
            .titles
            .iter()
            .filter(|(_, title)| q.is_empty() || title.to_lowercase().contains(&q))
            .map(|(id, title)| (title.clone(), id.clone()))
            .collect();
        out.sort_by(|a, b| {
            let a_starts = a.0.to_lowercase().starts_with(&q);
            let b_starts = b.0.to_lowercase().starts_with(&q);
            b_starts.cmp(&a_starts).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
        });
        out.truncate(12);
        out
    }

    /// Кто ссылается на страницу.
    pub fn backlinks_of(&self, id: &str) -> Vec<String> {
        self.backlinks.get(id).cloned().unwrap_or_default()
    }

    /// Исходящие цели страницы (разрезолвленные, без дублей).
    pub fn outgoing_of(&self, id: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .outgoing
            .get(id)
            .map(|targets| targets.iter().filter_map(|t| self.resolve(t)).collect())
            .unwrap_or_default();
        out.dedup();
        out
    }

    /// Все страницы — для графа.
    pub fn pages(&self) -> Vec<String> {
        let mut v: Vec<String> = self.titles.keys().cloned().collect();
        v.sort();
        v
    }
}

/// Сырые цели `[[...]]` в md-тексте; fenced-код и объекты
/// (`kanban:`/`gantt:`/`shape:`) пропускаются.
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
                if !target.is_empty() && !is_object_target(target) {
                    out.push(target.to_string());
                }
            }
            rest = &after[close + 2..];
        }
    }
    out
}

/// `kanban:<id>` / `gantt:<id>` — врезка объекта, `shape:<вид>` — векторный
/// примитив; `base:`/`canvas:` — объекты первых волн, которых больше нет.
/// Ничто из этого не ссылка на страницу: без этой проверки каждая фигура
/// висела бы в графе битой ссылкой.
pub fn is_object_target(target: &str) -> bool {
    ["kanban:", "gantt:", "shape:", "base:", "canvas:"].iter().any(|p| target.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::notes::project::PageNode;

    #[test]
    fn lexer_finds_links_and_skips_code_and_objects() {
        let md = "текст [[Раз]] и ![[Два|врезка]] ![[kanban:abc]] ![[gantt:x]] ![[shape:rect]]\n```\n[[не ссылка]]\n```\n[[Три]]";
        assert_eq!(lex_links(md), vec!["Раз", "Два", "Три"]);
    }

    #[test]
    fn index_resolves_and_backlinks() {
        let mut a = PageNode::new("Альфа");
        a.id = "a".into();
        let mut b = PageNode::new("Бета");
        b.id = "b".into();
        let tree = ProjectTree { version: 1, roots: vec![a, b] };
        let idx = VaultIndex::build(&tree, |id| match id {
            "a" => Some("см. [[бета]]".to_string()),
            _ => Some(String::new()),
        });
        assert_eq!(idx.resolve("Бета").as_deref(), Some("b"));
        assert_eq!(idx.resolve("id:a").as_deref(), Some("a"));
        assert_eq!(idx.backlinks_of("b"), vec!["a"]);
        assert_eq!(idx.outgoing_of("a"), vec!["b"]);
        assert_eq!(idx.title_of("a"), "Альфа");
        assert_eq!(idx.complete("бе")[0].1, "b");
    }
}
