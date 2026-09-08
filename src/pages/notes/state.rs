//! Реактивное состояние режима «Заметки».
//!
//! Проект — один `.syn`-файл ([`project`]); дерево страниц живёт в памяти
//! (`tree`) и пишется автосейвом по `tree_rev`. Страницы и объекты
//! (доски, диаграммы, карты, календари, графики) загружаются лениво и
//! держатся в пулах `pages`/`objects`
//! — у каждого своя ручка с сигналом ревизии, на который подписан автосейв.
//! Активная страница одна; плитка рейла одна на проект.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use syngui::core::{Point, Rect};
use syngui::prelude::*;
use syngui::widgets::input::document_editor::{DocGrid, DocLayout, DocOp, DocumentEditorHandle};

use crate::config::{now_millis, AppConfig};

use super::activity::{ActivityLog, ActivityLogHandle, LogEntry};
use super::autosave;
use super::calendar::model::{CalView, CalendarDoc, CalendarStore};
use super::calendar::{CalendarHandle, CalendarStoreHandle};
use super::chart::model::{ChartDoc, ChartKind};
use super::chart::ChartHandle;
use super::gantt::model::GanttDoc;
use super::gantt::GanttHandle;
use super::index::VaultIndex;
use super::kanban::model::KanbanDoc;
use super::kanban::KanbanHandle;
use super::mindmap::model::MindmapDoc;
use super::mindmap::MindmapHandle;
use super::gantt::calendar::{days_to_iso, parse_days};
use super::project::{self, PageGrid, PageLayout, PageNode, ProjectTree};
use super::reminders::{Reminder, ReminderStateHandle};

/// Загруженная страница: исходник для виджета (fingerprint стабилен между
/// перестройками) + ручка редактора (модель, ревизия, очередь операций).
#[derive(Clone)]
pub struct LivePage {
    pub id: String,
    pub source: Arc<String>,
    pub handle: DocumentEditorHandle,
}

impl LivePage {
    /// Текущий markdown: модель, если её правили, иначе исходник.
    pub fn markdown(&self) -> String {
        if self.handle.revision().get_untracked() > 0 {
            self.handle.serialize()
        } else {
            (*self.source).clone()
        }
    }
}

/// Виды объектов-примитивов (`![[<kind>:<id>]]`).
pub const OBJECT_KINDS: [&str; 5] = ["kanban", "gantt", "mindmap", "calendar", "chart"];

/// Загруженный объект-врезка.
#[derive(Clone)]
pub enum LiveObject {
    Kanban { id: String, handle: KanbanHandle },
    Gantt { id: String, handle: GanttHandle },
    Mindmap { id: String, handle: MindmapHandle },
    Calendar { id: String, handle: CalendarHandle },
    Chart { id: String, handle: ChartHandle },
}

impl LiveObject {
    pub fn id(&self) -> &str {
        match self {
            LiveObject::Kanban { id, .. }
            | LiveObject::Gantt { id, .. }
            | LiveObject::Mindmap { id, .. }
            | LiveObject::Calendar { id, .. }
            | LiveObject::Chart { id, .. } => id,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            LiveObject::Kanban { .. } => "kanban",
            LiveObject::Gantt { .. } => "gantt",
            LiveObject::Mindmap { .. } => "mindmap",
            LiveObject::Calendar { .. } => "calendar",
            LiveObject::Chart { .. } => "chart",
        }
    }

    /// Сериализация без сигналов (автосейв, копии).
    pub fn serialize(&self) -> String {
        match self {
            LiveObject::Kanban { handle, .. } => handle.serialize(),
            LiveObject::Gantt { handle, .. } => handle.serialize(),
            LiveObject::Mindmap { handle, .. } => handle.serialize(),
            LiveObject::Calendar { handle, .. } => handle.serialize(),
            LiveObject::Chart { handle, .. } => handle.serialize(),
        }
    }

    /// Ревизия документа объекта (автосейв).
    pub fn revision(&self) -> u64 {
        match self {
            LiveObject::Kanban { handle, .. } => handle.revision.get(),
            LiveObject::Gantt { handle, .. } => handle.revision.get(),
            LiveObject::Mindmap { handle, .. } => handle.revision.get(),
            LiveObject::Calendar { handle, .. } => handle.revision.get(),
            LiveObject::Chart { handle, .. } => handle.revision.get(),
        }
    }

    pub fn bundle_path(&self) -> String {
        project::object_path(self.kind(), self.id())
    }
}

/// Вкладки правой панели.
pub const TAB_PROPS: usize = 0;
pub const TAB_LINKS: usize = 1;
/// Вкладки левой панели.
pub const TAB_PAGES: usize = 0;
pub const TAB_BLOCKS: usize = 1;

/// Куда упадёт страница, которую тащат по дереву «Содержимое»: индикатор
/// рисуется по этому сигналу, сама вставка считается заново на отпускании
/// (`contents::drop_plan`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeDropHint {
    /// Линия над строкой `id` — вставить перед ней (как соседа).
    Line(String),
    /// Линия под последней строкой — в конец корня.
    Tail,
    /// Рамка на строке `id` — вложить внутрь.
    Into(String),
}

#[derive(Clone, Copy)]
pub struct NotesCtx {
    pub project_path: RwSignal<PathBuf>,
    /// Имя проекта — подпись плитки рейла (имя файла без расширения).
    pub project_title: RwSignal<String>,
    pub tree: RwSignal<Arc<ProjectTree>>,
    /// Ревизия дерева — подписка автосейва.
    pub tree_rev: RwSignal<u64>,
    /// Раскрытые узлы дерева.
    pub expanded: RwSignal<HashSet<String>>,
    /// Активная страница (id).
    pub active: RwSignal<Option<String>>,
    /// Показать граф связей вместо страницы.
    pub show_graph: RwSignal<bool>,
    pub pages: RwSignal<Vec<LivePage>>,
    pub objects: RwSignal<Vec<LiveObject>>,
    /// Единое хранилище событий календаря (лениво из `notes/calendar.json`).
    pub calendar: RwSignal<Option<CalendarStoreHandle>>,
    /// Журнал изменений проекта (лениво из `notes/log/`).
    pub activity: RwSignal<Option<ActivityLogHandle>>,
    /// Напоминания: настройки и что уже показано (лениво из
    /// `notes/reminders.json`), текущий список для колокольчика, попап.
    pub reminder_state: RwSignal<Option<ReminderStateHandle>>,
    pub reminders: RwSignal<Arc<Vec<Reminder>>>,
    pub reminders_open: RwSignal<bool>,
    pub reminders_anchor: RwSignal<Rect>,
    /// Окно просмотра картинки (вложение карточки): файл в кэше и подпись.
    pub viewer_open: RwSignal<bool>,
    pub viewer_file: RwSignal<Option<(PathBuf, String)>>,
    pub right_tab: RwSignal<usize>,
    /// Вкладка левой панели: 0 — «Содержимое», 1 — «Блоки».
    pub left_tab: RwSignal<usize>,
    pub index: RwSignal<Arc<VaultIndex>>,
    /// Тик перестройки блоков редактора после внешних правок модели
    /// (очередь DocOp, patch_media).
    pub doc_epoch: RwSignal<u64>,
    /// Общая ревизия объектов проекта: доски и диаграммы бампают её на
    /// каждую правку, а виджеты с чужими данными (календарь, Гант с
    /// досками) по ней пересобирают свой внешний слой.
    pub objects_rev: RwSignal<u64>,
    /// Плитка проекта на рейле: штамп открытия (None — закрыта).
    pub tile_opened_at: RwSignal<Option<u64>>,
    /// Строка дерева в режиме переименования.
    pub renaming: RwSignal<Option<String>>,
    /// Подсказка дропа в дереве страниц (см. [`TreeDropHint`]). `None` —
    /// ничего не тащат или курсор вне дерева.
    pub tree_drop: RwSignal<Option<TreeDropHint>>,
    /// Панель выбора иконки: страница-цель, якорь, открыта ли.
    pub icon_picker_page: RwSignal<Option<String>>,
    pub icon_picker_anchor: RwSignal<Rect>,
    pub icon_picker_open: RwSignal<bool>,
    /// Контекстное меню документа.
    pub doc_menu_open: RwSignal<bool>,
    pub doc_menu_pos: RwSignal<Point>,
}

impl NotesCtx {
    /// Открыть (или создать, мигрировав старую папку) проект и восстановить
    /// активную страницу, раскрытые узлы и плитку.
    pub fn new_or_restore(cfg: &AppConfig) -> Self {
        let path = project::resolve_project_path(&cfg.notes_project_path);
        if !path.exists() {
            let legacy = project::legacy_vault_path(&cfg.notes_vault_path);
            let migrated = project::migrate_folder(&legacy);
            let (tree, files) = migrated.unwrap_or_else(|| (ProjectTree::new(), Vec::new()));
            match project::create(&path, &tree, files) {
                Ok(()) => log::info!(
                    "notes: создан проект {} ({} страниц)",
                    path.display(),
                    tree.all().len()
                ),
                Err(e) => log::error!("notes: не удалось создать проект {}: {e}", path.display()),
            }
        } else {
            project::compact_if_needed(&path);
        }
        autosave::set_project_path(path.clone());
        let tree = project::read_tree(&path);
        let index = VaultIndex::build(&tree, |id| project::read_text(&path, &project::page_path(id)));

        let active = cfg
            .notes_active
            .clone()
            .filter(|id| tree.find(id).is_some())
            .or_else(|| tree.first_id());
        let mut expanded: HashSet<String> = cfg
            .notes_expanded
            .iter()
            .filter(|id| tree.find(id).is_some())
            .cloned()
            .collect();
        if let Some(id) = &active {
            for (pid, _) in tree.path_of(id) {
                if pid != *id {
                    expanded.insert(pid);
                }
            }
        }
        autosave::mark_saved(project::TREE_PATH, 0);
        Self {
            project_title: use_signal(project::project_title(&path)),
            project_path: use_signal(path),
            tree: use_signal(Arc::new(tree)),
            tree_rev: use_signal(0),
            objects_rev: use_signal(0),
            expanded: use_signal(expanded),
            active: use_signal(active),
            show_graph: use_signal(false),
            pages: use_signal(Vec::new()),
            objects: use_signal(Vec::new()),
            calendar: use_signal(None),
            activity: use_signal(None),
            reminder_state: use_signal(None),
            reminders: use_signal(Arc::new(Vec::new())),
            reminders_open: use_signal(false),
            reminders_anchor: use_signal(Rect::zero()),
            viewer_open: use_signal(false),
            viewer_file: use_signal(None),
            right_tab: use_signal(TAB_PROPS),
            left_tab: use_signal(TAB_PAGES),
            index: use_signal(Arc::new(index)),
            doc_epoch: use_signal(0),
            tile_opened_at: use_signal(cfg.notes_tile_opened_at),
            renaming: use_signal(None),
            tree_drop: use_signal(None),
            icon_picker_page: use_signal(None),
            icon_picker_anchor: use_signal(Rect::zero()),
            icon_picker_open: use_signal(false),
            doc_menu_open: use_signal(false),
            doc_menu_pos: use_signal(Point::zero()),
        }
    }

    // ─── Дерево ───────────────────────────────────────────────────────────

    fn edit_tree(&self, f: impl FnOnce(&mut ProjectTree)) {
        let mut t = (*self.tree.get_untracked()).clone();
        f(&mut t);
        self.tree.set(Arc::new(t));
        self.tree_rev.set(self.tree_rev.get_untracked() + 1);
        self.reindex_titles();
    }

    pub fn title_of(&self, id: &str) -> String {
        self.tree.get_untracked().title_of(id).unwrap_or_default()
    }

    pub fn toggle_expanded(&self, id: &str) {
        self.expanded.update(|set| {
            if !set.remove(id) {
                set.insert(id.to_string());
            }
        });
    }

    fn expand_ancestors(&self, id: &str) {
        let path = self.tree.get_untracked().path_of(id);
        self.expanded.update(|set| {
            for (pid, _) in path {
                if pid != id {
                    set.insert(pid);
                }
            }
        });
    }

    /// Уникальное среди соседей имя: «Название», «Название 2», …
    pub fn unique_title(&self, parent: Option<&str>, base: &str) -> String {
        let tree = self.tree.get_untracked();
        let siblings: Vec<String> = match parent {
            None => tree.roots.iter().map(|n| n.title.clone()).collect(),
            Some(pid) => tree
                .find(pid)
                .map(|n| n.children.iter().map(|c| c.title.clone()).collect())
                .unwrap_or_default(),
        };
        if !siblings.iter().any(|t| t == base) {
            return base.to_string();
        }
        (2..1000)
            .map(|i| format!("{base} {i}"))
            .find(|t| !siblings.contains(t))
            .unwrap_or_else(|| base.to_string())
    }

    /// Новая пустая страница у родителя (`None` — в корень); становится
    /// активной. Возвращает id.
    pub fn create_page(&self, parent: Option<&str>, base_title: &str) -> String {
        self.insert_page(parent, None, base_title, true)
    }

    /// Новая пустая страница у родителя на позиции `index` (`None` — в
    /// конец); название делается уникальным среди соседей. `activate` —
    /// показать её сразу (UI); агент создаёт страницы тихо, не уводя
    /// пользователя с того, что он читает. Возвращает id.
    pub fn insert_page(
        &self,
        parent: Option<&str>,
        index: Option<usize>,
        base_title: &str,
        activate: bool,
    ) -> String {
        let title = self.unique_title(parent, base_title);
        let node = PageNode::new(title.clone());
        let id = node.id.clone();
        self.edit_tree(|t| {
            t.insert(parent, index, node);
        });
        autosave::queue_bytes(&project::page_path(&id), Vec::new());
        self.activity().record(LogEntry::now("page", "create").item(id.clone()).title(title));
        if let Some(pid) = parent {
            self.expanded.update(|set| {
                set.insert(pid.to_string());
            });
        }
        if activate {
            self.activate(&id);
        }
        id
    }

    /// Страницы с таким названием (без регистра), в порядке дерева.
    pub fn find_by_title(&self, title: &str) -> Vec<String> {
        let key = title.trim().to_lowercase();
        self.tree
            .get_untracked()
            .all()
            .into_iter()
            .filter(|n| n.title.trim().to_lowercase() == key)
            .map(|n| n.id.clone())
            .collect()
    }

    /// Заменить markdown страницы целиком (агент, импорт). Модель ручки
    /// заменяется с записью в undo, исходник в пуле обновляется на тот же
    /// текст — иначе первый показ страницы перепарсил бы старый исходник
    /// поверх правки; смонтированный редактор перестраивается по
    /// `doc_epoch`, индекс связей обновляется сразу. `false` — страницы нет.
    pub fn set_page_markdown(&self, id: &str, md: &str) -> bool {
        let Some(page) = self.page(id) else { return false };
        page.handle.replace_markdown(md);
        let source = Arc::new(md.to_string());
        self.pages.update(|v| {
            if let Some(p) = v.iter_mut().find(|p| p.id == id) {
                p.source = source;
            }
        });
        self.reindex_page(id, md);
        self.bump_doc_epoch();
        true
    }

    /// Страницы, в которых врезан объект `kind:id`.
    pub fn pages_referencing(&self, kind: &str, id: &str) -> Vec<String> {
        self.tree
            .get_untracked()
            .all_ids()
            .into_iter()
            .filter(|pid| {
                object_refs(&self.page_markdown(pid)).iter().any(|(k, oid)| k == kind && oid == id)
            })
            .collect()
    }

    /// Удалить объект (доску/диаграмму) из проекта: файл уходит ближайшим
    /// коммитом, ручка — из пула. Врезки в страницах вызывающий убирает сам.
    pub fn delete_object(&self, kind: &str, id: &str) {
        autosave::queue_remove(&project::object_path(kind, id));
        self.objects.update(|v| v.retain(|o| o.id() != id));
    }

    pub fn rename_page(&self, id: &str, title: &str) {
        let title = title.trim();
        if title.is_empty() || self.title_of(id) == title {
            return;
        }
        let old = self.title_of(id);
        let title = title.to_string();
        self.edit_tree(|t| {
            if let Some(n) = t.find_mut(id) {
                n.title = title.clone();
            }
        });
        self.activity().record(LogEntry::now("page", "rename").item(id.to_string()).title(title.clone()).from_to(old, title));
    }

    pub fn set_icon(&self, id: &str, icon: Option<String>) {
        self.edit_tree(|t| {
            if let Some(n) = t.find_mut(id) {
                n.icon = icon.filter(|s| !s.is_empty());
            }
        });
    }

    /// Раскладка страницы: режим, сетка, привязка (панель «Свойства»).
    pub fn page_layout(&self, id: &str) -> PageLayout {
        self.tree.get_untracked().layout_of(id)
    }

    /// Изменить раскладку страницы; редактор перестроится по `doc_epoch`.
    pub fn set_page_layout(&self, id: &str, layout: PageLayout) {
        if self.page_layout(id) == layout {
            return;
        }
        self.edit_tree(|t| {
            if let Some(n) = t.find_mut(id) {
                n.layout = layout;
            }
        });
        self.bump_doc_epoch();
    }

    /// Правая панель свойств страницы: `(положение разделителя, скрыта)`.
    pub fn props_panel(&self, id: &str) -> (Option<f32>, bool) {
        let l = self.tree.get_untracked().layout_of(id);
        (l.props_ratio, l.props_hidden)
    }

    /// Запомнить панель свойств за страницей. Мимо `set_page_layout`:
    /// ширина панели к содержимому страницы отношения не имеет, и
    /// перестраивать по ней редактор (`doc_epoch`) незачем. Положение
    /// квантуется — иначе каждый пиксель перетаскивания разделителя
    /// поднимал бы ревизию дерева.
    pub fn set_props_panel(&self, id: &str, ratio: f32, hidden: bool) {
        let ratio = (ratio.clamp(0.05, 0.95) * 200.0).round() / 200.0;
        let cur = self.tree.get_untracked().layout_of(id);
        if cur.props_ratio == Some(ratio) && cur.props_hidden == hidden {
            return;
        }
        self.edit_tree(|t| {
            if let Some(n) = t.find_mut(id) {
                n.layout.props_ratio = Some(ratio);
                n.layout.props_hidden = hidden;
            }
        });
    }

    /// Раскладка активной страницы в терминах редактора.
    pub fn active_doc_layout(&self) -> DocLayout {
        let Some(id) = self.active.get() else { return DocLayout::default() };
        let l = self.tree.get().layout_of(&id);
        DocLayout {
            // Раскладка одна — холст: блоки стоят по координатам.
            free: true,
            grid: match l.grid {
                PageGrid::None => DocGrid::None,
                PageGrid::Dots => DocGrid::Dots,
                PageGrid::Lines => DocGrid::Lines,
                PageGrid::Cross => DocGrid::Cross,
            },
            grid_step: l.grid_step,
            snap: l.snap,
            snap_step: l.snap_step,
            background: (!l.bg.is_empty()).then(|| syngui::core::Color::from_hex(&l.bg)),
            ..DocLayout::default()
        }
    }

    /// Удалить страницу с поддеревом: файлы страниц и их объектов уходят
    /// из бандла ближайшим коммитом.
    pub fn delete_page(&self, id: &str) {
        let tree = self.tree.get_untracked();
        let doomed = tree.subtree_ids(id);
        if doomed.is_empty() {
            return;
        }
        let parent = tree.parent_of(id);
        let next = {
            let idx = tree.index_in_parent(id).unwrap_or(0);
            let siblings: Vec<String> = match &parent {
                None => tree.roots.iter().map(|n| n.id.clone()).collect(),
                Some(pid) => tree
                    .find(pid)
                    .map(|n| n.children.iter().map(|c| c.id.clone()).collect())
                    .unwrap_or_default(),
            };
            siblings
                .get(idx + 1)
                .or_else(|| idx.checked_sub(1).and_then(|i| siblings.get(i)))
                .cloned()
                .or(parent.clone())
                .or_else(|| tree.roots.iter().map(|n| n.id.clone()).find(|i| !doomed.contains(i)))
        };
        drop(tree);
        self.activity().record(
            LogEntry::now("page", "delete")
                .item(id.to_string())
                .title(self.title_of(id))
                .from_to(if doomed.len() > 1 { format!("{} sub-pages", doomed.len() - 1) } else { String::new() }, String::new()),
        );
        for pid in &doomed {
            let md = self.page_markdown(pid);
            for (kind, oid) in object_refs(&md) {
                let path = project::object_path(&kind, &oid);
                autosave::queue_remove(&path);
                self.objects.update(|v| v.retain(|o| o.id() != oid));
            }
            let path = project::page_path(pid);
            autosave::queue_remove(&path);
            self.pages.update(|v| v.retain(|p| p.id != *pid));
            self.expanded.update(|set| {
                set.remove(pid);
            });
        }
        self.edit_tree(|t| {
            t.remove(id);
        });
        if self
            .active
            .get_untracked()
            .as_deref()
            .map(|a| doomed.iter().any(|d| d == a))
            .unwrap_or(false)
        {
            match next {
                Some(n) => self.activate(&n),
                None => self.active.set(None),
            }
        }
    }

    /// Копия страницы с поддеревом рядом с оригиналом: содержимое и объекты
    /// клонируются с новыми id. Копия становится активной.
    pub fn duplicate_page(&self, id: &str) {
        self.duplicate_page_with(id, true);
    }

    /// То же, с выбором — показывать ли копию; возвращает id копии.
    pub fn duplicate_page_with(&self, id: &str, activate: bool) -> Option<String> {
        let tree = self.tree.get_untracked();
        let node = tree.find(id).cloned()?;
        let parent = tree.parent_of(id);
        let idx = tree.index_in_parent(id).unwrap_or(0);
        drop(tree);
        let mut map: Vec<(String, String)> = Vec::new();
        let mut copy = project::clone_subtree(&node, &mut map);
        copy.title = self.unique_title(parent.as_deref(), &format!("{} (копия)", node.title));
        for (old, new) in &map {
            let mut md = self.page_markdown(old);
            for (kind, oid) in object_refs(&md) {
                let new_oid = project::new_id();
                let src_path = project::object_path(&kind, &oid);
                let content = self
                    .objects
                    .get_untracked()
                    .iter()
                    .find(|o| o.id() == oid)
                    .map(|o| o.serialize())
                    .or_else(|| project::read_text(&self.project_path.get_untracked(), &src_path));
                if let Some(content) = content {
                    autosave::queue_bytes(&project::object_path(&kind, &new_oid), content.into_bytes());
                    md = md.replace(&format!("{kind}:{oid}"), &format!("{kind}:{new_oid}"));
                }
            }
            autosave::queue_bytes(&project::page_path(new), md.into_bytes());
        }
        let new_id = copy.id.clone();
        self.edit_tree(|t| {
            t.insert(parent.as_deref(), Some(idx + 1), copy);
        });
        if activate {
            self.activate(&new_id);
        }
        Some(new_id)
    }

    /// Перенос узла: к новому родителю (`None` — корень) на позицию
    /// `index` (`None` — в конец). В собственное поддерево — запрещено.
    pub fn move_page(&self, id: &str, new_parent: Option<&str>, index: Option<usize>) -> bool {
        let tree = self.tree.get_untracked();
        if let Some(np) = new_parent {
            if tree.is_ancestor_or_self(id, np) {
                return false;
            }
        }
        drop(tree);
        let mut ok = false;
        self.edit_tree(|t| {
            let Some(node) = t.remove(id) else { return };
            ok = t.insert(new_parent, index, node);
        });
        if ok {
            if let Some(np) = new_parent {
                self.expanded.update(|set| {
                    set.insert(np.to_string());
                });
            }
        }
        ok
    }

    // ─── Страницы ─────────────────────────────────────────────────────────

    /// Загруженная страница (ленивая загрузка из бандла).
    pub fn page(&self, id: &str) -> Option<LivePage> {
        if let Some(p) = self.pages.get_untracked().iter().find(|p| p.id == id) {
            return Some(p.clone());
        }
        self.tree.get_untracked().find(id)?;
        let path = self.project_path.get_untracked();
        let source = project::read_text(&path, &project::page_path(id)).unwrap_or_default();
        let page = LivePage { id: id.to_string(), source: Arc::new(source), handle: DocumentEditorHandle::new() };
        autosave::mark_saved(&project::page_path(id), 0);
        self.pages.update(|v| v.push(page.clone()));
        Some(page)
    }

    /// Текущий markdown страницы (для врезок, индексации, копий).
    pub fn page_markdown(&self, id: &str) -> String {
        if let Some(p) = self.pages.get_untracked().iter().find(|p| p.id == id) {
            return p.markdown();
        }
        project::read_text(&self.project_path.get_untracked(), &project::page_path(id))
            .unwrap_or_default()
    }

    /// Сделать страницу активной (и загрузить). Исходник страницы после
    /// загрузки не трогаем: модель живёт в её [`DocumentEditorHandle`] и
    /// переживает и смену страницы, и размонтирование вкладки.
    pub fn activate(&self, id: &str) {
        if self.page(id).is_none() {
            return;
        }
        self.show_graph.set(false);
        self.expand_ancestors(id);
        self.active.set(Some(id.to_string()));
    }

    pub fn active_page(&self) -> Option<LivePage> {
        let id = self.active.get()?;
        self.pages.get().into_iter().find(|p| p.id == id)
    }

    /// Операция над документом активной страницы (контекстное меню).
    pub fn doc_op(&self, op: DocOp) {
        let Some(page) = self.active_page() else { return };
        page.handle.queue_op(op);
        self.doc_epoch.set(self.doc_epoch.get_untracked() + 1);
    }

    pub fn bump_doc_epoch(&self) {
        self.doc_epoch.set(self.doc_epoch.get_untracked() + 1);
    }

    // ─── Объекты (доски/диаграммы) ────────────────────────────────────────

    /// Живой объект по виду и id: из пула либо из бандла.
    pub fn object(&self, kind: &str, id: &str) -> Option<LiveObject> {
        if let Some(o) = self.objects.get_untracked().iter().find(|o| o.id() == id) {
            return Some(o.clone());
        }
        let path = project::object_path(kind, id);
        let content = project::read_text(&self.project_path.get_untracked(), &path)?;
        let obj = match kind {
            "kanban" => {
                let handle = KanbanHandle::new(KanbanDoc::parse(&content).ok()?).with_log(id, self.activity()).with_project_rev(self.objects_rev);
                handle.sweep();
                LiveObject::Kanban { id: id.to_string(), handle }
            }
            "gantt" => LiveObject::Gantt {
                id: id.to_string(),
                handle: GanttHandle::new(GanttDoc::parse(&content).ok()?).with_project_rev(self.objects_rev),
            },
            "mindmap" => LiveObject::Mindmap {
                id: id.to_string(),
                handle: MindmapHandle::new(MindmapDoc::parse(&content).ok()?),
            },
            "calendar" => LiveObject::Calendar {
                id: id.to_string(),
                handle: CalendarHandle::new(CalendarDoc::parse(&content).ok()?),
            },
            "chart" => LiveObject::Chart {
                id: id.to_string(),
                handle: ChartHandle::new(ChartDoc::parse(&content).ok()?),
            },
            _ => return None,
        };
        autosave::mark_saved(&path, 0);
        self.objects.update(|v| v.push(obj.clone()));
        Some(obj)
    }

    /// Новый объект из шаблона; сразу пишется в бандл. Возвращает id.
    pub fn create_object(&self, kind: &str) -> Option<String> {
        let id = project::new_id();
        let (obj, content) = match kind {
            "kanban" => {
                let doc = KanbanDoc::template([
                    &tr!("notes.kanban.col.todo"),
                    &tr!("notes.kanban.col.doing"),
                    &tr!("notes.kanban.col.done"),
                ]);
                let content = doc.serialize();
                (LiveObject::Kanban { id: id.clone(), handle: KanbanHandle::new(doc).with_log(&id, self.activity()).with_project_rev(self.objects_rev) }, content)
            }
            "gantt" => {
                let doc = GanttDoc::template();
                let content = doc.serialize();
                (LiveObject::Gantt { id: id.clone(), handle: GanttHandle::new(doc).with_project_rev(self.objects_rev) }, content)
            }
            "mindmap" => {
                let doc = MindmapDoc::template(&tr!("notes.mindmap.root"));
                let content = doc.serialize();
                (LiveObject::Mindmap { id: id.clone(), handle: MindmapHandle::new(doc) }, content)
            }
            "calendar" => {
                let doc = CalendarDoc::template(CalView::Month, super::gantt::calendar::today_days());
                let content = doc.serialize();
                (LiveObject::Calendar { id: id.clone(), handle: CalendarHandle::new(doc) }, content)
            }
            "chart" => {
                let doc = ChartDoc::template(ChartKind::Line, &tr!("notes.chart.series"));
                let content = doc.serialize();
                (LiveObject::Chart { id: id.clone(), handle: ChartHandle::new(doc) }, content)
            }
            _ => return None,
        };
        self.register_object(kind, &id, obj, content);
        Some(id)
    }

    /// Виджет календаря с заданным видом.
    pub fn create_calendar(&self, view: CalView) -> String {
        let id = project::new_id();
        let doc = CalendarDoc::template(view, super::gantt::calendar::today_days());
        let content = doc.serialize();
        self.register_object("calendar", &id, LiveObject::Calendar { id: id.clone(), handle: CalendarHandle::new(doc) }, content);
        id
    }

    /// Хранилище событий проекта: из пула, из бандла либо новое (сразу
    /// пишется).
    pub fn calendar_store(&self) -> CalendarStoreHandle {
        if let Some(s) = self.calendar.get_untracked() {
            return s;
        }
        let path = self.project_path.get_untracked();
        let store = match project::read_text(&path, project::CALENDAR_PATH).and_then(|t| CalendarStore::parse(&t).ok()) {
            Some(s) => {
                autosave::mark_saved(project::CALENDAR_PATH, 0);
                s
            }
            None => {
                let s = CalendarStore::template(&tr!("notes.calendar.default_name"));
                autosave::mark_saved(project::CALENDAR_PATH, 0);
                autosave::queue_bytes(project::CALENDAR_PATH, s.serialize().into_bytes());
                s
            }
        };
        let handle = CalendarStoreHandle::new(store).with_log(self.activity());
        self.calendar.set(Some(handle.clone()));
        handle
    }

    /// Журнал изменений проекта: из пула либо из бандла.
    pub fn activity(&self) -> ActivityLogHandle {
        if let Some(a) = self.activity.get_untracked() {
            return a;
        }
        let handle = ActivityLogHandle::new(ActivityLog::load(&self.project_path.get_untracked()));
        self.activity.set(Some(handle.clone()));
        handle
    }

    // ─── Страницы дня ─────────────────────────────────────────────────────

    /// Страница дня `Журнал / yyyy-mm / yyyy-mm-dd`: находится либо
    /// создаётся вместе с родителями; не активируется. Корень — страница с
    /// названием журнала на любом из известных языков, иначе новая.
    pub fn journal_page(&self, day: i64) -> String {
        let root = self.journal_root();
        let (y, m, _) = super::gantt::calendar::civil_from_days(day);
        let month_title = format!("{y:04}-{m:02}");
        let month = self
            .child_titled(&root, &month_title)
            .unwrap_or_else(|| self.insert_page(Some(&root), None, &month_title, false));
        let day_title = days_to_iso(day);
        self.child_titled(&month, &day_title).unwrap_or_else(|| self.insert_page(Some(&month), None, &day_title, false))
    }

    /// Страница дня, если она уже есть.
    pub fn find_journal_page(&self, day: i64) -> Option<String> {
        let root = self.find_journal_root()?;
        let (y, m, _) = super::gantt::calendar::civil_from_days(day);
        let month = self.child_titled(&root, &format!("{y:04}-{m:02}"))?;
        self.child_titled(&month, &days_to_iso(day))
    }

    /// Корневая страница журнала (первая подходящая по названию).
    fn find_journal_root(&self) -> Option<String> {
        let tree = self.tree.get_untracked();
        let wanted = tr!("notes.journal.root").to_lowercase();
        tree.roots
            .iter()
            .find(|n| {
                let t = n.title.trim().to_lowercase();
                t == wanted || JOURNAL_ROOT_NAMES.contains(&t.as_str())
            })
            .map(|n| n.id.clone())
    }

    fn journal_root(&self) -> String {
        if let Some(id) = self.find_journal_root() {
            return id;
        }
        let id = self.insert_page(None, None, &tr!("notes.journal.root"), false);
        self.set_icon(&id, Some(crate::icons::MI_TODAY.to_string()));
        id
    }

    /// Ребёнок с таким названием (без регистра).
    fn child_titled(&self, parent: &str, title: &str) -> Option<String> {
        let tree = self.tree.get_untracked();
        let key = title.trim().to_lowercase();
        tree.find(parent)?.children.iter().find(|c| c.title.trim().to_lowercase() == key).map(|c| c.id.clone())
    }

    /// График заданного вида либо из готового документа (из таблицы
    /// страницы, от агента).
    pub fn create_chart(&self, doc: ChartDoc) -> String {
        let id = project::new_id();
        let content = doc.serialize();
        self.register_object("chart", &id, LiveObject::Chart { id: id.clone(), handle: ChartHandle::new(doc) }, content);
        id
    }

    /// Интеллект-карта из готового документа (из списка страницы, от агента).
    pub fn create_mindmap(&self, doc: MindmapDoc) -> String {
        let id = project::new_id();
        let content = doc.serialize();
        self.register_object("mindmap", &id, LiveObject::Mindmap { id: id.clone(), handle: MindmapHandle::new(doc) }, content);
        id
    }

    fn register_object(&self, kind: &str, id: &str, obj: LiveObject, content: String) {
        let path = project::object_path(kind, id);
        autosave::mark_saved(&path, 0);
        autosave::queue_bytes(&path, content.into_bytes());
        self.objects.update(|v| v.push(obj));
    }

    // ─── Индекс ───────────────────────────────────────────────────────────

    pub fn reindex_titles(&self) {
        let mut idx = (*self.index.get_untracked()).clone();
        idx.set_titles(&self.tree.get_untracked());
        self.index.set(Arc::new(idx));
    }

    pub fn reindex_page(&self, id: &str, content: &str) {
        let mut idx = (*self.index.get_untracked()).clone();
        idx.update_page(id, content);
        self.index.set(Arc::new(idx));
    }

    // ─── Плитка и персист ─────────────────────────────────────────────────

    pub fn open_tile(&self) {
        if self.tile_opened_at.get_untracked().is_none() {
            self.tile_opened_at.set(Some(now_millis()));
        }
    }

    pub fn close_tile(&self) {
        autosave::flush_all();
        self.tile_opened_at.set(None);
    }

    /// Снимок для конфига: активная страница, раскрытые узлы, плитка.
    pub fn persist(&self) -> (Option<String>, Vec<String>, Option<u64>) {
        let mut expanded: Vec<String> = self.expanded.get().into_iter().collect();
        expanded.sort();
        (self.active.get(), expanded, self.tile_opened_at.get())
    }
}

/// Названия корня журнала, которые узнаются на любом языке интерфейса.
pub const JOURNAL_ROOT_NAMES: [&str; 8] = ["журнал", "дневник", "journal", "diary", "daily", "daily notes", "tagebuch", "journal quotidien"];

/// Дата из названия страницы дня (`yyyy-mm-dd`) — ссылка `[[2026-09-08]]`.
pub fn date_title(title: &str) -> Option<i64> {
    let t = title.trim();
    (t.len() == 10 && t.as_bytes()[4] == b'-' && t.as_bytes()[7] == b'-').then(|| parse_days(t)).flatten()
}

/// `(kind, id)` всех врезок объектов в markdown: `![[kanban:<id>]]`.
pub fn object_refs(md: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = md;
    while let Some(pos) = rest.find("[[") {
        let after = &rest[pos + 2..];
        let Some(end) = after.find("]]") else { break };
        let inner = after[..end].trim();
        if let Some((kind, id)) = inner.split_once(':') {
            if OBJECT_KINDS.contains(&kind) && !id.is_empty() {
                let pair = (kind.to_string(), id.trim().to_string());
                if !out.contains(&pair) {
                    out.push(pair);
                }
            }
        }
        rest = &after[end + 2..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_refs_parse() {
        let md = "a ![[kanban:abc]] b [[Страница]] ![[gantt:xy]] ![[kanban:abc]] ![[base:old]]";
        assert_eq!(
            object_refs(md),
            vec![("kanban".to_string(), "abc".to_string()), ("gantt".to_string(), "xy".to_string())]
        );
    }
}
