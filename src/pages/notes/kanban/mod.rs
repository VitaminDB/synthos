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
//! `selected` — карточка, чьи поля (приоритет, срок, метки) показывают
//! панель свойств и ряд контролов под карточкой: правка текста может
//! закрыться по потере фокуса (клик по этим контролам), а выбор остаётся.
//! `hover` — место вставки под курсором во время переноса (плейсхолдер),
//! `drag_h` — высота переносимой карточки для него.
//! Карточка правится одним редактором: первый блок-заголовок (`## …`) —
//! её заголовок, остальное — содержимое ([`compose_card`] / [`split_card`]).

pub mod clip;
pub mod drag_strip;
#[cfg(all(test, feature = "testing"))]
mod harness_tests;
pub mod model;
pub mod sinks;
pub mod view;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use syngui::prelude::*;
use syngui::widgets::input::document_editor::DocumentEditorHandle;

use model::{item_id, next_color, CardChange, CardFile, DropSpot, KanbanCard, KanbanColumn, KanbanDoc, KanbanStyle, Priority, Repeat};

use super::activity::{ActivityLogHandle, LogEntry};
use super::gantt::calendar::today_days;

/// Поиск доски по id (перенос карточек между досками): в приложении — пул
/// объектов `NotesCtx`, в тестах — карта.
pub type Boards = Arc<dyn Fn(&str) -> Option<KanbanHandle> + Send + Sync>;

/// Блок страницы по payload'у drag'а (id блока): его markdown; блок при
/// этом уходит со страницы — он стал карточкой. `None` — блока нет.
pub type TakeBlock = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Файл вложения (`asset:…`) на диске — распакованный из бандла в кэш.
pub type AssetPath = Arc<dyn Fn(&str) -> Option<std::path::PathBuf> + Send + Sync>;
/// Файл с диска → вложение бандла (`None` — не прочитан).
pub type IngestFile = Arc<dyn Fn(&std::path::Path) -> Option<CardFile> + Send + Sync>;
/// Открыть вложение: картинку — в окне просмотра, файл — системой.
pub type OpenFile = Arc<dyn Fn(&CardFile) + Send + Sync>;
/// Диалог выбора файла → вложение бандла (`None` — отмена).
pub type PickFile = Arc<dyn Fn() -> Option<CardFile> + Send + Sync>;
/// Карточку — в буфер обмена в виде `CopyKind` (`&str` — имя её колонки).
pub type CopyCard = Arc<dyn Fn(&KanbanCard, &str, clip::CopyKind) + Send + Sync>;
/// Буфер обмена → карточка для вставки (`None` — вставлять нечего).
pub type PasteCard = Arc<dyn Fn() -> Option<KanbanCard> + Send + Sync>;

/// Окружение доски: другие доски, страница, на которой она врезана,
/// вложения (файлы бандла) и буфер обмена.
#[derive(Clone)]
pub struct BoardEnv {
    pub boards: Boards,
    pub take_block: TakeBlock,
    pub asset_path: AssetPath,
    pub ingest_file: IngestFile,
    pub open_file: OpenFile,
    pub pick_file: PickFile,
    pub copy_card: CopyCard,
    pub paste_card: PasteCard,
}

impl BoardEnv {
    /// Окружение без проекта (тесты): нет соседних досок, вложений и буфера.
    pub fn detached(boards: Boards, take_block: TakeBlock) -> Self {
        Self {
            boards,
            take_block,
            asset_path: Arc::new(|_| None),
            ingest_file: Arc::new(|_| None),
            open_file: Arc::new(|_| {}),
            pick_file: Arc::new(|| None),
            copy_card: Arc::new(|_, _, _| {}),
            paste_card: Arc::new(|| None),
        }
    }
}

/// Окружение из контекста заметок.
pub fn env(ctx: super::state::NotesCtx) -> BoardEnv {
    BoardEnv {
        boards: Arc::new(move |id| match ctx.object("kanban", id) {
            Some(super::state::LiveObject::Kanban { handle, .. }) => Some(handle),
            _ => None,
        }),
        take_block: Arc::new(move |payload| sinks::take_page_block(ctx, payload)),
        asset_path: Arc::new(move |url| super::media::asset_file(&ctx.project_path.get_untracked(), url)),
        ingest_file: Arc::new(move |path| super::media::ingest_card_file(ctx, path)),
        open_file: Arc::new(move |file| super::media::open_card_file(ctx, file)),
        pick_file: Arc::new(move || super::media::pick_card_file(ctx)),
        copy_card: Arc::new(move |card, column, kind| {
            if clip::copy_card(&ctx.project_path.get_untracked(), card, column, kind) {
                let msg =
                    if kind == clip::CopyKind::Files { tr!("notes.kanban.copied_files") } else { tr!("notes.kanban.copied") };
                use_context::<crate::context::AppCtx>().notifications.info(msg);
            }
        }),
        paste_card: Arc::new(move || {
            let card = clip::paste_card(&ctx.project_path.get_untracked());
            if card.is_none() {
                use_context::<crate::context::AppCtx>().notifications.info(tr!("notes.kanban.paste_empty"));
            }
            card
        }),
    }
}

#[derive(Clone)]
pub struct KanbanHandle {
    doc: Arc<Mutex<KanbanDoc>>,
    pub revision: RwSignal<u64>,
    pub structure_rev: RwSignal<u64>,
    /// Карточка в режиме правки.
    pub editing: RwSignal<Option<String>>,
    /// Выбранная карточка (поля в панели свойств и под карточкой).
    pub selected: RwSignal<Option<String>>,
    /// Место вставки под курсором во время переноса.
    pub hover: RwSignal<Option<DropSpot>>,
    /// Высота переносимой карточки (плейсхолдер); 0 — по умолчанию.
    pub drag_h: RwSignal<f32>,
    /// Редакторы карточек: ручка + исходник на момент начала правки.
    editors: Arc<Mutex<HashMap<String, (DocumentEditorHandle, Arc<String>)>>>,
    /// Журнал проекта и id доски в нём (`kanban:<id>`); без него правки
    /// не журналируются (тесты, доски вне проекта).
    log: Option<(String, ActivityLogHandle)>,
    /// Общая ревизия объектов проекта ([`super::state::NotesCtx`]).
    project_rev: Option<RwSignal<u64>>,
}

impl KanbanHandle {
    pub fn new(doc: KanbanDoc) -> Self {
        Self {
            doc: Arc::new(Mutex::new(doc)),
            revision: use_signal(0),
            structure_rev: use_signal(0),
            editing: use_signal(None),
            selected: use_signal(None),
            hover: use_signal(None),
            drag_h: use_signal(0.0),
            editors: Arc::new(Mutex::new(HashMap::new())),
            log: None,
            project_rev: None,
        }
    }

    /// Подключить журнал проекта: изменения карточек пишутся под
    /// `kanban:<id>`.
    pub fn with_log(mut self, id: &str, log: ActivityLogHandle) -> Self {
        self.log = Some((format!("kanban:{id}"), log));
        self
    }

    /// Общая ревизия объектов проекта: по ней календарь и Гант пересобирают
    /// свой слой чужих карточек.
    pub fn with_project_rev(mut self, rev: RwSignal<u64>) -> Self {
        self.project_rev = Some(rev);
        self
    }

    /// Доводка при загрузке (штампы, автоархив) — без записи в журнал
    /// «добавлений»: карточки не новые, а просто без штампа.
    pub fn sweep(&self) {
        let changes = self.lock().sweep(today_days());
        if !changes.is_empty() {
            self.bump();
            self.structure_rev.set(self.structure_rev.get_untracked() + 1);
            self.report(changes.into_iter().filter(|c| c.kind == model::CardChangeKind::Archived).collect());
        }
    }

    fn report(&self, changes: Vec<CardChange>) {
        let Some((object, log)) = &self.log else { return };
        if changes.is_empty() {
            return;
        }
        log.record_all(changes.into_iter().map(|c| {
            LogEntry::now("card", c.kind.key()).object(object.clone()).item(c.card).title(c.title).from_to(c.from, c.to)
        }));
    }

    pub fn lock(&self) -> MutexGuard<'_, KanbanDoc> {
        self.doc.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn serialize(&self) -> String {
        self.lock().serialize()
    }

    fn bump(&self) {
        self.revision.set(self.revision.get_untracked() + 1);
        if let Some(rev) = self.project_rev {
            rev.set(rev.get_untracked() + 1);
        }
    }

    /// Правка, меняющая вид доски: автосейв + перестройка. После правки —
    /// доводка ([`KanbanDoc::reconcile`]: штампы, повторы, автоархив) и
    /// запись изменений в журнал проекта.
    pub fn edit(&self, f: impl FnOnce(&mut KanbanDoc)) {
        let changes = self.edit_quiet(f);
        self.report(changes);
    }

    /// То же без журнала (изъятие карточки при переносе на другую доску —
    /// там запись сделает принимающая сторона).
    fn edit_quiet(&self, f: impl FnOnce(&mut KanbanDoc)) -> Vec<CardChange> {
        let changes = {
            let mut doc = self.lock();
            let before = doc.clone();
            f(&mut doc);
            doc.reconcile(&before, today_days())
        };
        self.bump();
        self.structure_rev.set(self.structure_rev.get_untracked() + 1);
        changes
    }

    /// Правка данных без перестройки (текст карточки по ходу набора).
    fn edit_data(&self, f: impl FnOnce(&mut KanbanDoc)) {
        f(&mut self.lock());
        self.bump();
    }

    // ─── Колонки ──────────────────────────────────────────────────────────

    pub fn add_column(&self, name: &str) -> String {
        let id = item_id("c");
        let column = KanbanColumn { id: id.clone(), name: name.to_string(), color: String::new(), width: None, done: false };
        self.edit(|doc| doc.columns.push(column));
        id
    }

    /// Флаг «готово» у колонки: карточки в ней закрываются штампом.
    pub fn set_column_done(&self, id: &str, done: bool) {
        let changed = self.lock().columns.iter().any(|c| c.id == id && c.done != done);
        if changed {
            self.edit(|doc| {
                if let Some(c) = doc.columns.iter_mut().find(|c| c.id == id) {
                    c.done = done;
                }
            });
        }
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

    pub fn card(&self, id: &str) -> Option<KanbanCard> {
        self.lock().card(id).cloned()
    }

    /// Новая пустая карточка в конце колонки; сразу в режиме правки.
    pub fn add_card(&self, column: &str) -> String {
        let id = item_id("k");
        let card = KanbanCard::new(id.clone(), column.to_string());
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

    pub fn set_priority(&self, id: &str, priority: Option<Priority>) {
        let changed = self.lock().cards.iter().any(|c| c.id == id && c.priority != priority);
        if changed {
            self.edit(|doc| {
                if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                    c.priority = priority;
                }
            });
        }
    }

    /// Срок в ISO `yyyy-mm-dd`; `None` — снять.
    pub fn set_due(&self, id: &str, due: Option<String>) {
        let due = due.filter(|d| !d.trim().is_empty());
        let changed = self.lock().cards.iter().any(|c| c.id == id && c.due != due);
        if changed {
            self.edit(|doc| {
                if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                    c.due = due;
                }
            });
        }
    }

    /// Оценка длительности в минутах; `None` — снять.
    pub fn set_duration(&self, id: &str, duration: Option<u32>) {
        let duration = duration.filter(|d| *d > 0);
        let changed = self.lock().cards.iter().any(|c| c.id == id && c.duration != duration);
        if changed {
            self.edit(|doc| {
                if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                    c.duration = duration;
                }
            });
        }
    }

    /// Плановое начало/конец: `yyyy-mm-dd` либо `yyyy-mm-ddThh:mm`.
    /// `None` — снять поле.
    pub fn set_schedule(&self, id: &str, start: Option<String>, end: Option<String>) {
        let clean = |v: Option<String>| v.filter(|d| !d.trim().is_empty());
        let (start, end) = (clean(start), clean(end));
        let changed = self.lock().cards.iter().any(|c| c.id == id && (c.start != start || c.end != end));
        if changed {
            self.edit(|doc| {
                if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                    c.start = start;
                    c.end = end;
                }
            });
        }
    }

    /// «В календарь»: начало — день `day` (и время), конец — по оценке.
    pub fn schedule_card(&self, id: &str, day: i64, min: Option<u32>) {
        self.edit(|doc| {
            if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                c.schedule_at(day, min);
            }
        });
    }

    /// Снять план карточки (срок и оценка остаются).
    pub fn unschedule_card(&self, id: &str) {
        let has = self.lock().cards.iter().any(|c| c.id == id && (c.start.is_some() || c.end.is_some()));
        if has {
            self.edit(|doc| {
                if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                    c.unschedule();
                }
            });
        }
    }

    /// Перенести полосу карточки на `delta` дней (перетаскивание в
    /// календаре и в Ганте).
    pub fn shift_card_schedule(&self, id: &str, delta: i64) {
        let movable = self.lock().cards.iter().any(|c| c.id == id && c.schedule().is_some());
        if delta != 0 && movable {
            self.edit(|doc| {
                if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                    c.shift_schedule(delta);
                }
            });
        }
    }

    /// Задать полосу карточки днями (растягивание кромки бара в Ганте).
    pub fn set_card_span(&self, id: &str, start_day: i64, end_day: i64) {
        self.edit(|doc| {
            if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                c.set_span_days(start_day, end_day);
            }
        });
    }

    pub fn set_repeat(&self, id: &str, repeat: Repeat) {
        let changed = self.lock().cards.iter().any(|c| c.id == id && c.repeat != repeat);
        if changed {
            self.edit(|doc| {
                if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                    c.repeat = repeat;
                }
            });
        }
    }

    /// Через сколько дней после закрытия карточки уходят в архив; `None` —
    /// никогда. Сразу применяется к уже закрытым.
    pub fn set_archive_after(&self, days: Option<u32>) {
        if self.lock().archive_after == days {
            return;
        }
        self.edit(|doc| doc.archive_after = days);
    }

    /// Карточка — в архив (закрывается, если открыта).
    pub fn archive_card(&self, id: &str) {
        self.editors.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        if self.editing.get_untracked().as_deref() == Some(id) {
            self.editing.set(None);
        }
        if self.selected.get_untracked().as_deref() == Some(id) {
            self.selected.set(None);
        }
        self.edit(|doc| {
            doc.archive_card(id, today_days());
        });
    }

    /// Карточка из архива — обратно на доску.
    pub fn unarchive_card(&self, id: &str) {
        let today = today_days();
        self.edit(|doc| {
            doc.unarchive_card(id, today);
        });
    }

    /// Все карточки архива — обратно на доску.
    pub fn unarchive_all(&self) {
        let ids: Vec<String> = self.lock().archive.iter().map(|c| c.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let today = today_days();
        self.edit(|doc| {
            for id in ids {
                doc.unarchive_card(&id, today);
            }
        });
    }

    /// Добавить вложение карточке (повтор той же ссылки не дублируется).
    pub fn add_file(&self, id: &str, file: CardFile) {
        let dup = self.lock().cards.iter().any(|c| c.id == id && c.files.iter().any(|f| f.url == file.url));
        if dup {
            return;
        }
        self.edit(|doc| {
            if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                c.files.push(file);
            }
        });
    }

    /// Убрать вложение по ссылке (файл в бандле остаётся до GC).
    pub fn remove_file(&self, id: &str, url: &str) {
        let has = self.lock().cards.iter().any(|c| c.id == id && c.files.iter().any(|f| f.url == url));
        if has {
            self.edit(|doc| {
                if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                    c.files.retain(|f| f.url != url);
                }
            });
        }
    }

    pub fn set_tags(&self, id: &str, tags: Vec<String>) {
        let changed = self.lock().cards.iter().any(|c| c.id == id && c.tags != tags);
        if changed {
            self.edit(|doc| {
                if let Some(c) = doc.cards.iter_mut().find(|c| c.id == id) {
                    c.tags = tags;
                }
            });
        }
    }

    pub fn delete_card(&self, id: &str) {
        self.editors.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        if self.editing.get_untracked().as_deref() == Some(id) {
            self.editing.set(None);
        }
        if self.selected.get_untracked().as_deref() == Some(id) {
            self.selected.set(None);
        }
        self.edit(|doc| doc.cards.retain(|c| c.id != id));
    }

    /// Перенос карточки (дроп) в место `spot`.
    pub fn move_card(&self, card: &str, spot: &DropSpot) {
        let own = self.lock().is_own_spot(card, spot);
        if own {
            return;
        }
        self.edit(|doc| {
            doc.move_card(card, &spot.column, spot.before.as_deref());
        });
    }

    /// Новая карточка из блока страницы в месте `spot`: строка-заголовок
    /// (или единственная простая строка) становится заголовком, остальное —
    /// содержимым.
    pub fn add_card_from_md(&self, spot: &DropSpot, md: &str) -> Option<String> {
        let md = md.trim();
        if md.is_empty() {
            return None;
        }
        let (title, body) = split_block(md);
        let id = item_id("k");
        let mut card = KanbanCard::new(id.clone(), spot.column.clone());
        card.title = title;
        card.md = body;
        self.edit(|doc| {
            doc.cards.push(card);
            doc.move_card(&id, &spot.column, spot.before.as_deref());
        });
        Some(id)
    }

    /// Забрать карточку с доски (перенос на другую доску или страницу).
    pub fn take_card(&self, id: &str) -> Option<KanbanCard> {
        if self.editing.get_untracked().as_deref() == Some(id) {
            self.finish_editing(id);
        }
        if self.selected.get_untracked().as_deref() == Some(id) {
            self.selected.set(None);
        }
        self.editors.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        let mut taken = None;
        self.edit_quiet(|doc| {
            if let Some(pos) = doc.cards.iter().position(|c| c.id == id) {
                taken = Some(doc.cards.remove(pos));
            }
        });
        taken
    }

    /// Положить чужую карточку в место `spot`.
    pub fn insert_card(&self, mut card: KanbanCard, spot: &DropSpot) {
        card.column = spot.column.clone();
        let id = card.id.clone();
        self.edit(|doc| {
            doc.cards.push(card);
            doc.move_card(&id, &spot.column, spot.before.as_deref());
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

    // ─── Перенос: плейсхолдер ─────────────────────────────────────────────

    /// Место вставки под курсором; своё место переносимой карточки
    /// (`payload` — `<доска>|<карточка>`) не подсвечивается.
    pub fn set_hover(&self, spot: Option<DropSpot>, board: &str, payload: &str) {
        let spot = spot.filter(|s| {
            let own = payload
                .split_once('|')
                .filter(|(b, _)| *b == board)
                .map(|(_, card)| self.lock().is_own_spot(card, s))
                .unwrap_or(false);
            !own
        });
        if self.hover.get_untracked() != spot {
            self.hover.set(spot);
        }
    }

    pub fn clear_hover(&self) {
        if self.hover.get_untracked().is_some() {
            self.hover.set(None);
        }
    }

    // ─── Выбор и правка ───────────────────────────────────────────────────

    pub fn select(&self, id: Option<&str>) {
        let id = id.map(str::to_string);
        if self.selected.get_untracked() != id {
            self.selected.set(id);
        }
    }

    /// Начать правку карточки; правившаяся до этого закрывается.
    pub fn start_editing(&self, id: &str) {
        if let Some(prev) = self.editing.get_untracked() {
            if prev != id {
                self.finish_editing(&prev);
            }
        }
        self.select(Some(id));
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
    /// с актуального текста. Выбор остаётся.
    pub fn finish_editing(&self, id: &str) {
        let empty = self.lock().cards.iter().any(|c| c.id == id && c.is_empty());
        self.editors.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        if self.editing.get_untracked().as_deref() == Some(id) {
            self.editing.set(None);
        }
        if empty {
            if self.selected.get_untracked().as_deref() == Some(id) {
                self.selected.set(None);
            }
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

/// Блок страницы → (заголовок, содержимое): строка-заголовок — заголовок,
/// единственная простая строка — тоже, иначе всё в содержимое.
fn split_block(md: &str) -> (String, String) {
    let (title, body) = split_card(md);
    if !title.is_empty() {
        return (title, body);
    }
    let one_line = !md.contains('\n');
    let plain = one_line
        && !md.starts_with(['-', '*', '>', '#', '|', '!', '`', '+'])
        && !md.starts_with(|c: char| c.is_ascii_digit());
    if plain {
        (md.to_string(), String::new())
    } else {
        (String::new(), md.to_string())
    }
}

/// Дроп карточки в место `spot`: в пределах доски — сдвиг, между досками —
/// изъятие у источника и вставка сюда. Payload — `<доска>|<карточка>`.
pub fn drop_card(boards: &Boards, payload: &str, to_board: &str, to: &KanbanHandle, spot: &DropSpot) {
    let Some((from_board, card)) = payload.split_once('|') else { return };
    if from_board == to_board {
        to.move_card(card, spot);
        return;
    }
    let Some(from) = boards(from_board) else { return };
    if let Some(card) = from.take_card(card) {
        to.insert_card(card, spot);
    }
}

/// Дроп блока страницы в место `spot`: блок становится карточкой и уходит
/// со страницы. `false` — блока нет или он пуст.
pub fn drop_block(take_block: &TakeBlock, payload: &str, to: &KanbanHandle, spot: &DropSpot) -> bool {
    let Some(md) = take_block(payload) else { return false };
    to.add_card_from_md(spot, &md).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(title: &str, md: &str) -> KanbanCard {
        let mut c = KanbanCard::new("k".into(), "c".into());
        c.title = title.into();
        c.md = md.into();
        c
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

    #[test]
    fn block_becomes_a_card_with_a_title() {
        assert_eq!(split_block("# Импорт Excel"), ("Импорт Excel".into(), "".into()));
        assert_eq!(split_block("Простая строка"), ("Простая строка".into(), "".into()));
        assert_eq!(split_block("- [ ] пункт"), ("".into(), "- [ ] пункт".into()));
        assert_eq!(split_block("## Заг\n\nтело"), ("Заг".into(), "тело".into()));
    }

    #[test]
    fn drop_between_boards_and_blocks() {
        let a = KanbanHandle::new(KanbanDoc::template(["A1", "A2", "A3"]));
        let b = KanbanHandle::new(KanbanDoc::template(["B1", "B2", "B3"]));
        let (a1, b2) = (a.lock().columns[0].id.clone(), b.lock().columns[1].id.clone());
        let ka = a.add_card_from_md(&DropSpot::end(&a1), "# Задача").unwrap();
        a.finish_editing(&ka);
        let map: HashMap<String, KanbanHandle> = [("a".to_string(), a.clone()), ("b".to_string(), b.clone())].into();
        let boards: Boards = Arc::new(move |id| map.get(id).cloned());
        drop_card(&boards, &format!("a|{ka}"), "b", &b, &DropSpot::end(&b2));
        assert!(a.lock().cards.is_empty());
        assert_eq!(b.lock().cards_of(&b2).len(), 1);
        assert_eq!(b.lock().cards[0].title, "Задача");
        // Дроп своей карточки в своё же место — без изменений ревизии.
        let rev = b.revision.get_untracked();
        drop_card(&boards, &format!("b|{ka}"), "b", &b, &DropSpot::before(&b2, &ka));
        assert_eq!(b.revision.get_untracked(), rev);
        // Блок страницы.
        let take: TakeBlock = Arc::new(|p| (p == "7").then(|| "Из страницы\n".to_string()));
        assert!(drop_block(&take, "7", &b, &DropSpot::before(&b2, &ka)));
        assert!(!drop_block(&take, "8", &b, &DropSpot::end(&b2)));
        let titles: Vec<String> = b.lock().cards_of(&b2).iter().map(|c| c.title.clone()).collect();
        assert_eq!(titles, ["Из страницы", "Задача"]);
        // Плейсхолдер не встаёт на своё место карточки.
        b.set_hover(Some(DropSpot::before(&b2, &ka)), "b", &format!("b|{}", b.lock().cards[0].id));
        assert_eq!(b.hover.get_untracked(), None);
        b.set_hover(Some(DropSpot::end(&b2)), "b", "a|чужая");
        assert_eq!(b.hover.get_untracked(), Some(DropSpot::end(&b2)));
    }
}
