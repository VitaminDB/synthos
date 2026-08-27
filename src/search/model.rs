//! Типы индекса: что за элемент найден, как он выглядит в выдаче и что
//! произойдёт по Enter.
//!
//! `SearchItem` собирается билдером: заголовок ищется всегда, а всё, что
//! добавлено через `subtitle`/`hint`/`keyword`, попадает в дополнительные
//! поля — по ним элемент тоже находится, и панель показывает, по чему
//! именно совпало.

use std::path::PathBuf;

use super::matching;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SearchKind {
    Chat,
    Message,
    Page,
    Command,
    Model,
    Skill,
    Tool,
    Template,
}

impl SearchKind {
    pub const ALL: [SearchKind; 8] = [
        SearchKind::Chat,
        SearchKind::Message,
        SearchKind::Page,
        SearchKind::Command,
        SearchKind::Model,
        SearchKind::Skill,
        SearchKind::Tool,
        SearchKind::Template,
    ];

    pub fn label_key(self) -> &'static str {
        match self {
            SearchKind::Chat => "search.group.chats",
            SearchKind::Message => "search.group.messages",
            SearchKind::Page => "search.group.pages",
            SearchKind::Command => "search.group.commands",
            SearchKind::Model => "search.group.models",
            SearchKind::Skill => "search.group.skills",
            SearchKind::Tool => "search.group.tools",
            SearchKind::Template => "search.group.templates",
        }
    }

    /// Порядок групп в выдаче при равном счёте.
    pub fn rank(self) -> usize {
        Self::ALL
            .iter()
            .position(|k| *k == self)
            .unwrap_or(Self::ALL.len())
    }

    /// Класс подложки под иконкой строки — задаёт цвет плитки по типу.
    pub fn icon_box_class(self) -> &'static str {
        match self {
            SearchKind::Chat => "search-icon-box search-icon-box-chat",
            SearchKind::Message => "search-icon-box search-icon-box-message",
            SearchKind::Page => "search-icon-box search-icon-box-page",
            SearchKind::Command => "search-icon-box search-icon-box-command",
            SearchKind::Model => "search-icon-box search-icon-box-model",
            SearchKind::Skill => "search-icon-box search-icon-box-skill",
            SearchKind::Tool => "search-icon-box search-icon-box-tool",
            SearchKind::Template => "search-icon-box search-icon-box-template",
        }
    }

    /// Класс самой иконки — цвет глифа в тон подложке.
    pub fn icon_class(self) -> &'static str {
        match self {
            SearchKind::Chat => "search-row-icon search-row-icon-chat",
            SearchKind::Message => "search-row-icon search-row-icon-message",
            SearchKind::Page => "search-row-icon search-row-icon-page",
            SearchKind::Command => "search-row-icon search-row-icon-command",
            SearchKind::Model => "search-row-icon search-row-icon-model",
            SearchKind::Skill => "search-row-icon search-row-icon-skill",
            SearchKind::Tool => "search-row-icon search-row-icon-tool",
            SearchKind::Template => "search-row-icon search-row-icon-template",
        }
    }
}

/// Действия, у которых нет собственной сущности в индексе: тумблеры
/// оформления, окно, работа с активным чатом и моделью.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchCommand {
    NewChat,
    ClearChat,
    CompactNow,
    DeleteChat,
    LoadModel,
    UnloadModel,
    NewNodeTab,
    TemplatePicker,
    NewCodeSession,
    VoicePanel,
    DarkTheme,
    LightTheme,
    SystemTheme,
    NextLanguage,
    Fullscreen,
    Glass,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SearchAction {
    /// Маршрут верхнего уровня (`context::ROUTES`).
    Page(&'static str),
    /// Подраздел настроек (`context::SETTINGS_ROUTES`).
    Settings(&'static str),
    /// Открыть чат по id.
    Chat(String),
    /// Открыть чат и подсветить в нём сообщение по индексу в ленте.
    Message { chat_id: String, index: usize },
    /// Настройки → Скилы, с выделением скила.
    Skill(String),
    /// Чат → правая панель «Инструменты», с подсветкой чипа.
    Tool(String),
    /// Включить/выключить инструмент, не уходя из поиска.
    ToggleTool(String),
    /// Открыть шаблон графа новой вкладкой node-редактора.
    Template(String),
    /// Открыть `.syn` пакет в SynExplorer.
    Model(PathBuf),
    /// Поднять `.syn` бандл как модель чата.
    LoadModelFile(PathBuf),
    Command(SearchCommand),
}

/// Поле элемента, по которому идёт поиск помимо названия: нормализованный
/// текст для сравнения, исходный — для показа «по чему нашли».
#[derive(Clone, Debug, PartialEq)]
pub struct SearchField {
    pub norm: String,
    pub display: String,
    pub bonus: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchItem {
    pub key: String,
    pub kind: SearchKind,
    pub title: String,
    pub norm_title: String,
    pub subtitle: String,
    pub hint: String,
    pub icon: &'static str,
    pub fields: Vec<SearchField>,
    pub action: SearchAction,
    /// Действие по Shift+Enter, если у элемента есть второй сценарий.
    pub secondary: Option<SearchAction>,
}

impl SearchItem {
    pub fn new(
        key: impl Into<String>,
        kind: SearchKind,
        title: impl Into<String>,
        action: SearchAction,
    ) -> Self {
        let title = title.into();
        Self {
            key: key.into(),
            kind,
            norm_title: matching::normalize(&title),
            title,
            subtitle: String::new(),
            hint: String::new(),
            icon: "",
            fields: Vec::new(),
            action,
            secondary: None,
        }
    }

    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        let subtitle = subtitle.into();
        self.push_field(&subtitle, matching::SUBTITLE_BONUS);
        self.subtitle = subtitle;
        self
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        let hint = hint.into();
        self.push_field(&hint, 0);
        self.hint = hint;
        self
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = icon;
        self
    }

    pub fn keyword(mut self, keyword: impl AsRef<str>) -> Self {
        self.push_field(keyword.as_ref(), 0);
        self
    }

    pub fn keywords<I, S>(mut self, keywords: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for k in keywords {
            self.push_field(k.as_ref(), 0);
        }
        self
    }

    pub fn secondary(mut self, action: SearchAction) -> Self {
        self.secondary = Some(action);
        self
    }

    fn push_field(&mut self, text: &str, bonus: i32) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.fields.push(SearchField {
            norm: matching::normalize(text),
            display: text.to_string(),
            bonus,
        });
    }
}
