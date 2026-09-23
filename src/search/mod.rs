//! Глобальный поиск по приложению.
//!
//! Пилюля в шапке чата (или Ctrl+K / Ctrl+F на любой странице) раскрывает
//! панель с полем ввода: страницы и разделы настроек, чаты, сообщения из
//! всех чатов, `.syn`-пакеты, скилы, инструменты, шаблоны графов и команды
//! ищутся одним запросом — с подсветкой совпадений, исправлением раскладки
//! и клавиатурной навигацией.
//!
//! Индекс собирается эффектом из сигналов приложения; то, что лежит на
//! диске (сообщения чатов, `.syn`-файлы, шаблоны), сканируется фоновой
//! задачей при каждом открытии панели — см. [`SearchCtx::rescan`].

pub mod actions;
pub mod index;
pub mod matching;
pub mod model;
pub mod panel;
pub mod trigger;

use std::sync::Arc;

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::core::sync::Mutex;
use syngui::core::{Point, Rect, Size};
use syngui::input::{Key, Modifiers};
use syngui::prelude::*;
use syngui::widgets::containers::IntoWidget;

use crate::components::event_hook::{EventHook, KeyReply};
use crate::config;

pub use index::ScanData;
pub use model::{SearchItem, SearchKind};

pub const PANEL_WIDTH: f32 = 660.0;
pub const PANEL_MAX_HEIGHT: f32 = 720.0;
pub const PANEL_INSET: f32 = 8.0;
/// Строк (заголовков и результатов), которые панель показывает без
/// прокрутки; более длинная выдача идёт в бокс фиксированной высоты.
pub const VISIBLE_LINES: usize = 9;
pub const GROUP_LIMIT: usize = 5;
pub const FILTERED_LIMIT: usize = 60;
pub const RECENT_LIMIT: usize = 8;
const PAGE_STEP: i32 = 5;
/// Отступ панели от верхнего края окна, когда пилюли-триггера на экране
/// нет (поиск открыт с ноды, редактора кода и т.п.).
const FLOATING_TOP: f32 = 72.0;

/// Строка панели, на которую можно встать курсором.
#[derive(Clone)]
pub enum RowAction {
    Item(SearchItem),
    More(SearchKind),
}

/// Снимок видимых строк и групп — заполняется при построении панели,
/// читается обработчиком клавиш.
#[derive(Default)]
pub struct PanelState {
    pub rows: Vec<RowAction>,
    pub kinds: Vec<SearchKind>,
}

#[derive(Clone)]
pub struct SearchCtx {
    pub open: RwSignal<bool>,
    pub query: RwSignal<String>,
    pub selected: RwSignal<usize>,
    pub kind_filter: RwSignal<Option<SearchKind>>,
    pub anchor: RwSignal<Rect>,
    /// Растёт при каждом открытии: панель пересоздаёт поле ввода, и оно
    /// забирает фокус.
    pub nonce: RwSignal<u64>,
    pub index: RwSignal<Arc<Vec<SearchItem>>>,
    /// То, что прочитано с диска (сообщения, `.syn`, шаблоны).
    pub scan: RwSignal<Arc<ScanData>>,
    /// Идёт ли фоновое сканирование прямо сейчас.
    pub scanning: RwSignal<bool>,
    pub recent: RwSignal<Vec<String>>,
    /// Геометрия пилюли в шапке чата — к ней прижимается панель.
    pub trigger_bounds: Arc<Mutex<Rect>>,
    /// Геометрия оболочки окна — запасной якорь, когда пилюли нет.
    pub root_bounds: Arc<Mutex<Rect>>,
    pub state: Arc<Mutex<PanelState>>,
}

pub fn install() -> SearchCtx {
    let recent = config::AppConfig::load().search_recent;
    let search = SearchCtx {
        open: use_signal(false),
        query: use_signal(String::new()),
        selected: use_signal(0usize),
        kind_filter: use_signal(None::<SearchKind>),
        anchor: use_signal(Rect::zero()),
        nonce: use_signal(0u64),
        index: use_signal(Arc::new(Vec::new())),
        scan: use_signal(Arc::new(ScanData::default())),
        scanning: use_signal(false),
        recent: use_signal(recent),
        trigger_bounds: Arc::new(Mutex::new(Rect::zero())),
        root_bounds: Arc::new(Mutex::new(Rect::zero())),
        state: Arc::new(Mutex::new(PanelState::default())),
    };
    provide_context(search.clone());

    let index = search.index;
    let scan = search.scan;
    create_effect(move || {
        // Подписка на язык интерфейса: заголовки страниц, команд и групп
        // берутся из каталогов, значит индекс пересобирается при смене.
        syngui::i18n::subscribe();
        let scanned = scan.get();
        let items = index::build_items(&scanned);
        index.set_always(Arc::new(items));
    });
    install_debug_open(&search);
    search
}

/// `SYNTHOS_SEARCH_OPEN=<запрос>` — отладочный хук: вскоре после старта
/// открыть панель с готовым запросом. Нужен для скриншотов и ручной
/// проверки выдачи без кликов.
fn install_debug_open(search: &SearchCtx) {
    let Ok(query) = std::env::var("SYNTHOS_SEARCH_OPEN") else {
        return;
    };
    let search = search.clone();
    spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        run_on_main_thread(move || search.open(Some(query)));
    });
}

impl SearchCtx {
    pub fn open(&self, prefill: Option<String>) {
        self.anchor.set(self.panel_anchor());
        self.query.set(prefill.unwrap_or_default());
        self.selected.set(0);
        self.kind_filter.set(None);
        self.nonce.update(|n| *n += 1);
        self.open.set(true);
        self.rescan();
    }

    pub fn close(&self) {
        self.open.set(false);
    }

    pub fn toggle(&self) {
        if self.open.get_untracked() {
            self.close();
        } else {
            self.open(None);
        }
    }

    pub fn set_filter(&self, kind: Option<SearchKind>) {
        self.kind_filter.set(kind);
        self.selected.set(0);
    }

    fn row_count(&self) -> usize {
        self.state.lock().map(|s| s.rows.len()).unwrap_or(0)
    }

    pub fn move_selection(&self, delta: i32) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let current = self.selected.get_untracked().min(count - 1) as i32;
        let next = (current + delta).rem_euclid(count as i32);
        self.selected.set(next as usize);
    }

    pub fn select_edge(&self, end: bool) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        self.selected.set(if end { count - 1 } else { 0 });
    }

    pub fn cycle_filter(&self, delta: i32) {
        let kinds: Vec<SearchKind> = self
            .state
            .lock()
            .map(|s| s.kinds.clone())
            .unwrap_or_default();
        if kinds.is_empty() {
            return;
        }
        let mut options: Vec<Option<SearchKind>> = Vec::with_capacity(kinds.len() + 1);
        options.push(None);
        options.extend(kinds.into_iter().map(Some));
        let current = self.kind_filter.get_untracked();
        let pos = options.iter().position(|o| *o == current).unwrap_or(0) as i32;
        let next = (pos + delta).rem_euclid(options.len() as i32) as usize;
        self.set_filter(options[next]);
    }

    pub fn activate(&self, secondary: bool) {
        let row = {
            let Ok(state) = self.state.lock() else {
                return;
            };
            if state.rows.is_empty() {
                return;
            }
            let idx = self.selected.get_untracked().min(state.rows.len() - 1);
            state.rows[idx].clone()
        };
        match row {
            RowAction::Item(item) => actions::run(self, &item, secondary),
            RowAction::More(kind) => self.set_filter(Some(kind)),
        }
    }

    /// Запомнить выбранный элемент в списке недавних (новые — в начале).
    pub fn remember(&self, key: &str) {
        let mut recent = self.recent.get_untracked();
        recent.retain(|k| k != key);
        recent.insert(0, key.to_string());
        recent.truncate(RECENT_LIMIT);
        self.recent.set(recent.clone());
        config::AppConfig::update(|cfg| cfg.search_recent = recent);
    }

    /// Перечитать диск: сообщения всех чатов, `.syn`-пакеты и шаблоны.
    /// Дёргается при каждом открытии панели — чат, который правили минуту
    /// назад, должен находиться по своему последнему сообщению.
    pub fn rescan(&self) {
        if self.scanning.get_untracked() {
            return;
        }
        self.scanning.set(true);
        let scan = self.scan;
        let scanning = self.scanning;
        let dirs = index::scan_dirs();
        // Открытый чат мог ещё не уйти на диск: автосейв ждёт паузы.
        crate::syn_chat::autosave::flush();
        spawn(async move {
            crate::syn_chat::storage::flush_all();
            let data = index::scan_disk(&dirs);
            run_on_main_thread(move || {
                scan.set_always(Arc::new(data));
                scanning.set(false);
            });
        });
    }

    /// Панель прижимается левым краем к пилюле в шапке чата, а верхом — к
    /// её верхней границе: поле ввода ложится ровно на пилюлю. Если пилюли
    /// на экране нет (другая страница), панель центрируется в окне.
    fn panel_anchor(&self) -> Rect {
        let trigger = self
            .trigger_bounds
            .lock()
            .map(|b| *b)
            .unwrap_or_else(|_| Rect::zero());
        if trigger.size.width > 1.0 {
            return Rect::new(
                Point::new(trigger.origin.x, trigger.origin.y - PANEL_INSET),
                Size::zero(),
            );
        }
        let root = self
            .root_bounds
            .lock()
            .map(|b| *b)
            .unwrap_or_else(|_| Rect::zero());
        let x = ((root.size.width - PANEL_WIDTH) * 0.5).max(0.0);
        Rect::new(Point::new(x, FLOATING_TOP), Size::zero())
    }
}

/// Клавиши открытой панели. Вызывается и обёрткой-предком, и невидимым
/// последним ребёнком панели — кто первым увидит нажатие, тот и обработает.
pub fn handle_panel_key(search: &SearchCtx, key: Key, mods: Modifiers) -> KeyReply {
    match key {
        Key::Escape => {
            search.close();
            KeyReply::Handled
        }
        Key::K | Key::F if mods.ctrl => {
            search.close();
            KeyReply::Handled
        }
        Key::Down => {
            search.move_selection(1);
            KeyReply::Handled
        }
        Key::Up => {
            search.move_selection(-1);
            KeyReply::Handled
        }
        Key::PageDown => {
            search.move_selection(PAGE_STEP);
            KeyReply::Handled
        }
        Key::PageUp => {
            search.move_selection(-PAGE_STEP);
            KeyReply::Handled
        }
        Key::Home if mods.ctrl => {
            search.select_edge(false);
            KeyReply::Handled
        }
        Key::End if mods.ctrl => {
            search.select_edge(true);
            KeyReply::Handled
        }
        Key::Right if mods.ctrl => {
            search.cycle_filter(1);
            KeyReply::Handled
        }
        Key::Left if mods.ctrl => {
            search.cycle_filter(-1);
            KeyReply::Handled
        }
        Key::Enter => {
            search.activate(mods.shift);
            KeyReply::Handled
        }
        Key::Backspace
            if search.query.get_untracked().is_empty()
                && search.kind_filter.get_untracked().is_some() =>
        {
            search.set_filter(None);
            KeyReply::Handled
        }
        _ => KeyReply::Ignore,
    }
}

/// Обёртка оболочки окна: Ctrl+K и Ctrl+F открывают и закрывают поиск на
/// любой странице. Она же сообщает панели размер окна — по нему считается
/// запасной якорь, когда пилюли в шапке на экране нет.
pub fn hotkey_scope<M>(child: impl IntoWidget<M>) -> impl Widget {
    let search = use_context::<SearchCtx>();
    let bounds = search.root_bounds.clone();
    EventHook::new()
        .report_bounds(bounds)
        .on_key_down(move |key, mods| {
            if matches!(key, Key::K | Key::F) && mods.ctrl {
                search.toggle();
                KeyReply::Handled
            } else {
                KeyReply::Ignore
            }
        })
        .child(child)
}
