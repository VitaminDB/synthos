//! Канбан-доска — объект страницы: живая врезка `![[kanban:<id>]]` над
//! `notes/objects/<id>.kanban.json`.
//!
//! [`KanbanHandle`] держит документ (Mutex) и сигналы: `revision` растёт
//! на каждую правку (автосейв), `structure_rev` — только на те, что меняют
//! вид доски (перестройка виджета): набор в карточке стекает в документ по
//! `revision`, не пересоздавая редактор на каждую букву. `editing` —
//! карточка, которую сейчас правят; её редактор живёт в `editors` вместе
//! с исходником на момент начала правки (стабильный отпечаток для
//! `DocumentEditor`, иначе каждая перестройка перепарсивала бы модель).
//! Карточка правится одним редактором: первый блок-заголовок (`## …`) —
//! её заголовок, остальное — содержимое ([`compose_card`] / [`split_card`]).

pub mod drag_strip;
pub mod model;
pub mod sinks;
pub mod view;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use syngui::prelude::*;
use syngui::widgets::input::document_editor::DocumentEditorHandle;

use model::{item_id, next_color, KanbanCard, KanbanColumn, KanbanDoc, KanbanStyle};

#[derive(Clone)]
pub struct KanbanHandle {
    doc: Arc<Mutex<KanbanDoc>>,
    pub revision: RwSignal<u64>,
    pub structure_rev: RwSignal<u64>,
    /// Карточка в режиме правки.
    pub editing: RwSignal<Option<String>>,
    /// Редакторы карточек: ручка + исходник на момент начала правки.
    editors: Arc<Mutex<HashMap<String, (DocumentEditorHandle, Arc<String>)>>>,
}

impl KanbanHandle {
    pub fn new(doc: KanbanDoc) -> Self {
        Self {
            doc: Arc::new(Mutex::new(doc)),
            revision: use_signal(0),
            structure_rev: use_signal(0),
            editing: use_signal(None),
            editors: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, KanbanDoc> {
        self.doc.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn serialize(&self) -> String {
        self.lock().serialize()
    }

    fn bump(&self) {
        self.revision.set(self.revision.get_untracked() + 1);
    }

    /// Правка, меняющая вид доски: автосейв + перестройка.
    pub fn edit(&self, f: impl FnOnce(&mut KanbanDoc)) {
        f(&mut self.lock());
        self.bump();
        self.structure_rev.set(self.structure_rev.get_untracked() + 1);
    }

    /// Правка данных без перестройки (текст карточки по ходу набора).
    fn edit_data(&self, f: impl FnOnce(&mut KanbanDoc)) {
        f(&mut self.lock());
        self.bump();
    }

    // ─── Колонки ──────────────────────────────────────────────────────────

    pub fn add_column(&self, name: &str) -> String {
        let id = item_id("c");
        let column = KanbanColumn { id: id.clone(), name: name.to_string(), color: String::new(), width: None };
        self.edit(|doc| doc.columns.push(column));
        id
    }

    pub fn rename_column(&self, id: &str, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let changed = self.lock().columns.iter().any(|c| c.id == id && c.name != name);
        if changed {
            self.edit(|doc| {
                if let Some(c) = doc.columns.iter_mut().find(|c| c.id == id) {
                    c.name = name.to_string();
                }
            });
        }
    }

    /// Следующий цвет палитры у метки колонки.
    pub fn cycle_column_color(&self, id: &str) {
        self.edit(|doc| {
            if let Some(c) = doc.columns.iter_mut().find(|c| c.id == id) {
                c.color = next_color(&c.color).to_string();
            }
        });
    }

    /// Текущая ширина колонки (своя либо общая).
    pub fn column_width(&self, id: &str) -> f32 {
        let doc = self.lock();
        doc.columns.iter().find(|c| c.id == id).map(|c| doc.column_width(c)).unwrap_or(doc.style.column_width)
    }

    /// Своя ширина колонки; `None` — вернуть к общей.
    pub fn set_column_width(&self, id: &str, width: Option<f32>) {
        let width = width.map(|w| w.clamp(model::MIN_COLUMN_WIDTH, model::MAX_COLUMN_WIDTH).round());
        let same = self.lock().columns.iter().any(|c| c.id == id && c.width == width);
        if same {
            return;
        }
        self.edit(|doc| {
            if let Some(c) = doc.columns.iter_mut().find(|c| c.id == id) {
                c.width = width;
            }
        });
    }

    /// Удалить колонку; её карточки переезжают в соседнюю (слева, иначе
    /// справа), а без соседей — удаляются вместе с ней.
    pub fn delete_column(&self, id: &str) {
        self.edit(|doc| {
            let Some(idx) = doc.columns.iter().position(|c| c.id == id) else { return };
            let heir = idx
                .checked_sub(1)
                .or_else(|| (idx + 1 < doc.columns.len()).then_some(idx + 1))
                .map(|i| doc.columns[i].id.clone());
            doc.columns.remove(idx);
            match heir {
                Some(h) => doc.cards.iter_mut().filter(|c| c.column == id).for_each(|c| c.column = h.clone()),
                None => doc.cards.retain(|c| c.column != id),
            }
        });
    }

    // ─── Внешний вид ──────────────────────────────────────────────────────

    pub fn style(&self) -> KanbanStyle {
        self.lock().style.clone()
    }

    pub fn set_style(&self, f: impl FnOnce(&mut KanbanStyle)) {
        let mut style = self.style();
        f(&mut style);
        style.column_width = style.column_width.clamp(model::MIN_COLUMN_WIDTH, model::MAX_COLUMN_WIDTH).round();
        if self.lock().style == style {
            return;
        }
        self.edit(|doc| doc.style = style);
    }

    // ─── Карточки ─────────────────────────────────────────────────────────

    /// Новая пустая карточка в конце колонки; сразу в режиме правки.
    pub fn add_card(&self, column: &str) -> String {
        let id = item_id("k");
        let card = KanbanCard { id: id.clone(), column: column.to_string(), title: String::new(), md: String::new() };
        self.edit(|doc| doc.cards.push(card));
        self.start_editing(&id);
        id
    }

    pub fn set_card_title(&self, id: &str, title: &str) {
        let title = title.trim().to_string();
        let changed = self.lock().cards.iter().any(|c| c.id == id && c.title != title);
        if changed {
            self.edit_data(|doc| {
                if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                    c.title = title;
                }
            });
        }
    }

    pub fn delete_card(&self, id: &str) {
        self.editors.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        if self.editing.get_untracked().as_deref() == Some(id) {
            self.editing.set(None);
        }
        self.edit(|doc| doc.cards.retain(|c| c.id != id));
    }

    /// Перенос карточки (дроп): в колонку перед `before` либо в её конец.
    pub fn move_card(&self, card: &str, column: &str, before: Option<&str>) {
        let mut moved = false;
        self.edit(|doc| moved = doc.move_card(card, column, before));
        let _ = moved;
    }

    /// Дописать markdown в содержимое карточки (дроп блока страницы).
    pub fn append_card_md(&self, id: &str, md: &str) -> bool {
        let md = md.trim_end();
        if md.is_empty() {
            return false;
        }
        let exists = self.lock().cards.iter().any(|c| c.id == id);
        if !exists {
            return false;
        }
        // Правящаяся карточка держит свой редактор — его исходник устарел бы.
        if self.editing.get_untracked().as_deref() == Some(id) {
            self.finish_editing(id);
        }
        self.edit(|doc| {
            if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                if c.md.trim().is_empty() {
                    c.md = md.to_string();
                } else {
                    c.md = format!("{}\n\n{md}", c.md.trim_end());
                }
            }
        });
        true
    }

    /// Новая карточка из блока страницы: текстовый блок в одну строку
    /// становится заголовком, остальное — содержимым.
    pub fn add_card_from_md(&self, column: &str, md: &str) -> Option<String> {
        let md = md.trim();
        if md.is_empty() {
            return None;
        }
        let one_line = !md.contains('\n');
        let plain = one_line
            && !md.starts_with(['-', '*', '>', '#', '|', '!', '`'])
            && !md.starts_with(|c: char| c.is_ascii_digit());
        let (title, body) = if plain { (md.to_string(), String::new()) } else { (String::new(), md.to_string()) };
        let id = item_id("k");
        let card = KanbanCard { id: id.clone(), column: column.to_string(), title, md: body };
        self.edit(|doc| doc.cards.push(card));
        Some(id)
    }

    /// Забрать карточку с доски (перенос на другую доску или страницу).
    pub fn take_card(&self, id: &str) -> Option<KanbanCard> {
        if self.editing.get_untracked().as_deref() == Some(id) {
            self.finish_editing(id);
        }
        self.editors.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        let mut taken = None;
        self.edit(|doc| {
            if let Some(pos) = doc.cards.iter().position(|c| c.id == id) {
                taken = Some(doc.cards.remove(pos));
            }
        });
        taken
    }

    /// Положить чужую карточку в колонку перед `before` (либо в конец).
    pub fn insert_card(&self, mut card: KanbanCard, column: &str, before: Option<&str>) {
        card.column = column.to_string();
        let id = card.id.clone();
        self.edit(|doc| {
            doc.cards.push(card);
            doc.move_card(&id, column, before);
        });
    }

    /// Карточка как блоки страницы: заголовок третьего уровня + содержимое.
    pub fn card_markdown(card: &KanbanCard) -> String {
        let title = card.title.trim();
        let md = card.md.trim();
        match (title.is_empty(), md.is_empty()) {
            (true, true) => String::new(),
            (true, false) => format!("{md}\n"),
            (false, true) => format!("### {title}\n"),
            (false, false) => format!("### {title}\n\n{md}\n"),
        }
    }

    /// Начать правку карточки; правившаяся до этого закрывается.
    pub fn start_editing(&self, id: &str) {
        if let Some(prev) = self.editing.get_untracked() {
            if prev != id {
                self.finish_editing(&prev);
            }
        }
        self.editing.set(Some(id.to_string()));
    }

    /// Редактор карточки: ручка и исходник на момент начала правки. Правки
    /// стекают в `md` документа эффектом по ревизии ручки.
    pub fn card_editor(&self, id: &str) -> (DocumentEditorHandle, Arc<String>) {
        let mut editors = self.editors.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = editors.get(id) {
            return e.clone();
        }
        let source = Arc::new(
            self.lock().cards.iter().find(|c| c.id == id).map(compose_card).unwrap_or_default(),
        );
        let editor = DocumentEditorHandle::new();
        editors.insert(id.to_string(), (editor.clone(), source.clone()));
        drop(editors);
        let h = self.clone();
        let card_id = id.to_string();
        let e = editor.clone();
        create_effect(move || {
            if e.revision().get() == 0 {
                return;
            }
            let (title, md) = split_card(&e.serialize());
            let changed = h.lock().cards.iter().any(|c| c.id == card_id && (c.md != md || c.title != title));
            if changed {
                h.edit_data(|doc| {
                    if let Some(c) = doc.cards.iter_mut().find(|c| c.id == card_id) {
                        c.title = title;
                        c.md = md;
                    }
                });
            }
        });
        (editor, source)
    }

    /// Закончить правку: пустая карточка выбрасывается, чтобы на доске не
    /// копились безымянные; редактор забывается — следующая правка начнёт
    /// с актуального текста.
    pub fn finish_editing(&self, id: &str) {
        let empty = self.lock().cards.iter().any(|c| c.id == id && c.is_empty());
        self.editors.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        if self.editing.get_untracked().as_deref() == Some(id) {
            self.editing.set(None);
        }
        if empty {
            self.edit(|doc| doc.cards.retain(|c| c.id != id));
        } else {
            // Вид карточки в просмотре — по свежему тексту.
            self.structure_rev.set(self.structure_rev.get_untracked() + 1);
        }
    }
}

/// Исходник редактора карточки: заголовок — блок `## …` первой строкой,
/// содержимое — ниже. У пустой карточки — пустой заголовок, чтобы каретка
/// сразу стояла в нём.
pub fn compose_card(card: &KanbanCard) -> String {
    let title = card.title.trim();
    let md = card.md.trim_end();
    match (title.is_empty(), md.is_empty()) {
        (true, true) => "## \n".to_string(),
        (true, false) => format!("{md}\n"),
        (false, true) => format!("## {title}\n"),
        (false, false) => format!("## {title}\n\n{md}\n"),
    }
}

/// Обратно: первая строка-заголовок любого уровня — заголовок карточки,
/// остальное (без ведущих пустых строк) — содержимое.
pub fn split_card(source: &str) -> (String, String) {
    let mut lines = source.lines();
    let Some(first) = lines.next() else { return (String::new(), String::new()) };
    let hashes = first.chars().take_while(|c| *c == '#').count();
    let is_heading = (1..=6).contains(&hashes) && first[hashes..].starts_with(' ');
    if !is_heading && !(hashes >= 1 && first.len() == hashes) {
        return (String::new(), source.trim().to_string());
    }
    let title = first[hashes..].trim().to_string();
    let rest: Vec<&str> = lines.collect();
    let body = rest.join("\n");
    (title, body.trim().to_string())
}

/// Перенос карточки по дропу: в пределах доски — сдвиг, между досками —
/// изъятие у источника и вставка сюда. Payload — `<доска>|<карточка>`.
pub fn drop_card(
    ctx: super::state::NotesCtx,
    payload: &str,
    to_board: &str,
    to: &KanbanHandle,
    column: &str,
    before: Option<&str>,
) {
    let Some((from_board, card)) = payload.split_once('|') else { return };
    if from_board == to_board {
        to.move_card(card, column, before);
        return;
    }
    let Some(super::state::LiveObject::Kanban { handle: from, .. }) = ctx.object("kanban", from_board) else {
        return;
    };
    if let Some(card) = from.take_card(card) {
        to.insert_card(card, column, before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(title: &str, md: &str) -> KanbanCard {
        KanbanCard { id: "k".into(), column: "c".into(), title: title.into(), md: md.into() }
    }

    #[test]
    fn compose_and_split_roundtrip() {
        for (t, m) in [("Заголовок", "- [ ] пункт\n- [x] готово"), ("", "просто текст"), ("Только заголовок", ""), ("", "")] {
            let src = compose_card(&card(t, m));
            let (bt, bm) = split_card(&src);
            assert_eq!((bt.as_str(), bm.as_str()), (t, m), "исходник: {src:?}");
        }
        // Пустой заголовок в редакторе, набранное содержимое — заголовка нет.
        assert_eq!(split_card("## \n\nтекст\n"), ("".into(), "текст".into()));
        assert_eq!(KanbanHandle::card_markdown(&card("З", "т")), "### З\n\nт\n");
    }
}
