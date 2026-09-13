//! Сборка индекса: что вообще можно найти.
//!
//! Делится надвое. Всё, что живёт в сигналах (чаты, скилы, инструменты,
//! страницы, команды), собирается на main-thread в [`build_items`] — это
//! дёшево и пересобирается эффектом при любом изменении. Всё, что лежит на
//! диске (тела сообщений, `.syn`-пакеты, шаблоны графов), читает [`scan_disk`]
//! в фоновой задаче: пути для него снимает [`scan_dirs`], потому что
//! сигналы доступны только на main-thread'е.

use std::collections::HashSet;
use std::path::PathBuf;

use syngui::prelude::*;

use crate::agent::storage::StoredChat;
use crate::agent::tools::Tool;
use crate::context::AppCtx;
use crate::icons::*;
use crate::skills::Skill;
use crate::syn_chat::attach::format_size;
use crate::syn_chat::state::{ChatMsgKind, ChatMsgRole};
use crate::syn_chat::SynChatCtx;
use crate::templates::{self, TemplateCategory};

use super::model::{SearchAction, SearchCommand, SearchItem, SearchKind};

/// Потолок длины индексируемого тела сообщения. Вывод `bash`-инструмента
/// бывает и на мегабайт — держать такое в индексе целиком незачем: и поиск
/// по первым строкам находит нужный чат, и память остаётся конечной.
const MAX_MSG_CHARS: usize = 600;
/// Общий потолок числа проиндексированных сообщений.
const MAX_MESSAGES: usize = 6000;
/// Потолок числа `.syn`-пакетов в выдаче.
const MAX_MODELS: usize = 500;

/// Пути, снятые с сигналов на main-thread'е для фонового сканера.
#[derive(Clone, Default, PartialEq)]
pub struct ScanDirs {
    pub chats_dir: PathBuf,
    /// Каталоги, где ищутся `.syn`-пакеты: общий каталог моделей плюс
    /// закладки SynExplorer.
    pub model_dirs: Vec<PathBuf>,
}

/// Прочитанное с диска. Живёт в `SearchCtx.scan`, обновляется при каждом
/// открытии панели.
#[derive(Clone, Default, PartialEq)]
pub struct ScanData {
    pub messages: Vec<MessageEntry>,
    pub models: Vec<ModelEntry>,
    pub templates: Vec<TemplateEntry>,
}

#[derive(Clone, PartialEq)]
pub struct MessageEntry {
    pub chat_id: String,
    pub chat_title: String,
    /// Индекс в ленте чата — по нему сообщение подсвечивается после перехода.
    pub index: usize,
    pub author: String,
    pub time: String,
    /// Тело, обрезанное до [`MAX_MSG_CHARS`].
    pub text: String,
}

#[derive(Clone, PartialEq)]
pub struct ModelEntry {
    pub path: PathBuf,
    pub name: String,
    pub folder: String,
    pub bytes: u64,
}

#[derive(Clone, PartialEq)]
pub struct TemplateEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Ключ раздела builtin-шаблона; пусто у пользовательских.
    pub category_key: String,
    pub builtin: bool,
}

/// Маршруты верхнего уровня в порядке показа. `music` из `context::ROUTES`
/// сюда не входит — страницы за ним пока нет.
const PAGES: &[(&str, &str, &str)] = &[
    ("syn_chat", "nav.syn_chat", MI_CHAT),
    ("nodes", "nav.nodes", MI_HUB),
    ("notes", "notes.title", MI_EDIT_NOTE),
    ("code", "search.page.code", MI_CODE),
    ("syn_explorer", "nav.syn_explorer", MI_INVENTORY_2),
    ("huggingface", "nav.huggingface", MI_CLOUD_DOWNLOAD),
    ("settings", "nav.settings", MI_SETTINGS),
];

/// Разделы настроек — ключи совпадают с `context::SETTINGS_ROUTES`.
const SETTINGS: &[(&str, &str)] = &[
    ("general", MI_TUNE),
    ("themes", MI_PALETTE),
    ("skills", MI_PSYCHOLOGY),
    ("audio_models", MI_HEADSET_MIC),
    ("ai_models", MI_AUTO_AWESOME),
    ("knowledge_base", MI_MENU_BOOK),
    ("terminal", MI_TERMINAL),
    ("archive", MI_INBOX),
    ("about", MI_INFO),
];

fn page_keywords(route: &str) -> &'static [&'static str] {
    match route {
        "syn_chat" => &["чат", "чаты", "переписка", "диалог", "сообщения", "chat", "messages"],
        "notes" => &["заметки", "заметка", "vault", "wiki", "страницы", "notes", "markdown"],
        "nodes" => &[
            "ноды",
            "нодовый редактор",
            "граф",
            "пайплайн",
            "видео",
            "музыка",
            "nodes",
            "graph",
            "pipeline",
        ],
        "code" => &[
            "редактор кода",
            "код",
            "файлы",
            "терминал",
            "git",
            "code",
            "editor",
            "terminal",
        ],
        "syn_explorer" => &[
            "syn",
            "пакеты",
            "бандлы",
            "модели",
            "explorer",
            "bundles",
            "packages",
        ],
        "huggingface" => &["hf", "загрузки", "скачать", "hub", "download", "huggingface"],
        "settings" => &["настройки", "параметры", "опции", "settings", "preferences"],
        _ => &[],
    }
}

fn settings_keywords(key: &str) -> &'static [&'static str] {
    match key {
        "general" => &[
            "язык",
            "системный промпт",
            "подтверждение инструментов",
            "автосжатие",
            "шрифт",
            "general",
            "language",
            "system prompt",
        ],
        "themes" => &[
            "тема",
            "тёмная",
            "светлая",
            "оформление",
            "цвет",
            "акцент",
            "стекло",
            "прозрачность",
            "theme",
            "appearance",
        ],
        "skills" => &["скилы", "инструкции", "autoskill", "skills"],
        "audio_models" => &["asr", "whisper", "gigaam", "распознавание речи", "микрофон", "audio"],
        "ai_models" => &["квантование", "llm", "qwen", "muse", "attention", "квант", "models"],
        "knowledge_base" => &["rag", "база знаний", "коллекции", "эмбеддинги", "kb", "knowledge"],
        "terminal" => &["терминал", "шрифт терминала", "vte", "terminal", "font"],
        "archive" => &["архив", "закрытые чаты", "восстановить", "archive", "archived", "restore"],
        "about" => &["о программе", "версия", "лицензия", "about", "version"],
        _ => &[],
    }
}

struct CommandSpec {
    command: SearchCommand,
    key: &'static str,
    icon: &'static str,
    keywords: &'static [&'static str],
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        command: SearchCommand::NewChat,
        key: "search.cmd.new_chat",
        icon: MI_ADD,
        keywords: &["новый чат", "создать чат", "new chat"],
    },
    CommandSpec {
        command: SearchCommand::CompactNow,
        key: "search.cmd.compact",
        icon: MI_COMPRESS,
        keywords: &["сжать", "контекст", "автосжатие", "compact", "context"],
    },
    CommandSpec {
        command: SearchCommand::ClearChat,
        key: "search.cmd.clear_chat",
        icon: MI_CLEAR_ALL,
        keywords: &["очистить", "стереть ленту", "clear"],
    },
    CommandSpec {
        command: SearchCommand::DeleteChat,
        key: "search.cmd.archive_chat",
        icon: MI_ARCHIVE,
        keywords: &["удалить чат", "delete chat"],
    },
    CommandSpec {
        command: SearchCommand::LoadModel,
        key: "search.cmd.load_model",
        icon: MI_POWER_SETTINGS,
        keywords: &["загрузить модель", "поднять модель", "load model"],
    },
    CommandSpec {
        command: SearchCommand::UnloadModel,
        key: "search.cmd.unload_model",
        icon: MI_POWER_SETTINGS,
        keywords: &["выгрузить", "освободить vram", "unload model"],
    },
    CommandSpec {
        command: SearchCommand::NewNodeTab,
        key: "search.cmd.new_node_tab",
        icon: MI_HUB,
        keywords: &["новый граф", "вкладка нод", "new graph"],
    },
    CommandSpec {
        command: SearchCommand::TemplatePicker,
        key: "search.cmd.templates",
        icon: MI_ACCOUNT_TREE,
        keywords: &["шаблоны", "пресеты", "templates"],
    },
    CommandSpec {
        command: SearchCommand::NewCodeSession,
        key: "search.cmd.new_code_session",
        icon: MI_NOTE_ADD,
        keywords: &["новая сессия", "открыть папку", "new session"],
    },
    CommandSpec {
        command: SearchCommand::VoicePanel,
        key: "search.cmd.voice",
        icon: MI_MIC,
        keywords: &[
            "голос",
            "голосовой ввод",
            "диктовка",
            "запись",
            "микрофон",
            "распознавание речи",
            "voice",
            "dictation",
            "speech",
        ],
    },
    CommandSpec {
        command: SearchCommand::DarkTheme,
        key: "search.cmd.dark_theme",
        icon: MI_DARK_MODE,
        keywords: &["тёмная тема", "ночная", "dark"],
    },
    CommandSpec {
        command: SearchCommand::LightTheme,
        key: "search.cmd.light_theme",
        icon: MI_LIGHT_MODE,
        keywords: &["светлая тема", "дневная", "light"],
    },
    CommandSpec {
        command: SearchCommand::SystemTheme,
        key: "search.cmd.system_theme",
        icon: MI_DESKTOP_WINDOWS,
        keywords: &["как в системе", "системная тема", "system theme"],
    },
    CommandSpec {
        command: SearchCommand::NextLanguage,
        key: "search.cmd.language",
        icon: MI_TRANSLATE,
        keywords: &["язык", "перевод", "language"],
    },
    CommandSpec {
        command: SearchCommand::Fullscreen,
        key: "search.cmd.fullscreen",
        icon: MI_FULLSCREEN,
        keywords: &["во весь экран", "f11", "fullscreen"],
    },
    CommandSpec {
        command: SearchCommand::Glass,
        key: "search.cmd.glass",
        icon: MI_BLUR_ON,
        keywords: &["стекло", "размытие", "blur", "glass"],
    },
];

/// Снимок путей для фонового сканера. Зовётся с main-thread'а.
pub fn scan_dirs() -> ScanDirs {
    let app = use_context::<AppCtx>();
    let mut model_dirs = vec![crate::config::resolve_models_dir(&app.models_dir.get_untracked())];
    let explorer = use_context::<crate::pages::syn_explorer::SynExplorerCtx>();
    for dir in explorer.bookmarks.get_untracked() {
        if !model_dirs.contains(&dir) {
            model_dirs.push(dir);
        }
    }
    ScanDirs {
        chats_dir: crate::syn_chat::storage::syn_chats_dir(),
        model_dirs,
    }
}

/// Чтение диска. Зовётся из фоновой задачи — сигналов здесь нет.
pub fn scan_disk(dirs: &ScanDirs) -> ScanData {
    ScanData {
        messages: scan_messages(&dirs.chats_dir),
        models: scan_models(&dirs.model_dirs),
        templates: scan_templates(),
    }
}

fn scan_messages(dir: &PathBuf) -> Vec<MessageEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<MessageEntry> = Vec::new();
    let mut chats: Vec<StoredChat> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(chat) = serde_json::from_str::<StoredChat>(&text) {
            if !chat.archived {
                chats.push(chat);
            }
        }
    }
    // Свежие чаты индексируются первыми: если упрёмся в потолок, обрежется
    // хвост из давно не открывавшихся.
    chats.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    for chat in chats {
        for (index, msg) in chat.messages.iter().enumerate() {
            if out.len() >= MAX_MESSAGES {
                return out;
            }
            // Ищем по человеческой переписке: tool-вызовы и их вывод — это
            // машинный шум, из-за которого выдача забивается одинаковыми
            // строками, а полезное сообщение уходит вниз.
            if msg.kind != ChatMsgKind::Text || msg.role == ChatMsgRole::System {
                continue;
            }
            let text = truncate(msg.body.trim(), MAX_MSG_CHARS);
            if text.is_empty() {
                continue;
            }
            out.push(MessageEntry {
                chat_id: chat.id.clone(),
                chat_title: chat.title.clone(),
                index,
                author: msg.author.clone(),
                time: msg.time.clone(),
                text,
            });
        }
    }
    out
}

fn scan_models(dirs: &[PathBuf]) -> Vec<ModelEntry> {
    let mut out: Vec<ModelEntry> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("syn") {
                continue;
            }
            if !seen.insert(path.clone()) {
                continue;
            }
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let folder = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(ModelEntry {
                path,
                name,
                folder,
                bytes,
            });
            if out.len() >= MAX_MODELS {
                return out;
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn scan_templates() -> Vec<TemplateEntry> {
    templates::list_all()
        .into_iter()
        .map(|t| TemplateEntry {
            category_key: if t.builtin {
                TemplateCategory::for_template(&t).key().to_string()
            } else {
                String::new()
            },
            id: t.id,
            name: t.name,
            description: t.description,
            builtin: t.builtin,
        })
        .collect()
}

fn truncate(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(max.min(s.len()));
    for (i, c) in s.chars().enumerate() {
        if i >= max {
            out.push('…');
            break;
        }
        // Переводы строк в одну строку: превью показывается одной строкой,
        // а искать «слово\nслово» по двум абзацам всё равно бессмысленно.
        out.push(if c == '\n' || c == '\r' { ' ' } else { c });
    }
    out.trim().to_string()
}

/// Полный индекс. Зовётся с main-thread'а внутри эффекта — читает сигналы.
pub fn build_items(scan: &ScanData) -> Vec<SearchItem> {
    let mut items = Vec::new();
    push_pages(&mut items);
    push_commands(&mut items);
    push_chats(&mut items);
    push_messages(&mut items, scan);
    push_models(&mut items, scan);
    push_skills(&mut items);
    push_tools(&mut items);
    push_templates(&mut items, scan);
    push_notes(&mut items);
    items
}

/// Страницы проекта заметок: название + путь в дереве, действие —
/// активировать страницу.
fn push_notes(items: &mut Vec<SearchItem>) {
    let notes = use_context::<crate::pages::notes::NotesCtx>();
    let tree = notes.tree.get();
    for node in tree.all() {
        let path = tree
            .path_of(&node.id)
            .into_iter()
            .map(|(_, t)| t)
            .collect::<Vec<_>>()
            .join(" / ");
        let icon = match node.icon.as_deref() {
            Some(i) if crate::pages::notes::icon_picker::is_material_glyph(i) => {
                Box::leak(i.to_string().into_boxed_str()) as &'static str
            }
            _ => MI_EDIT_NOTE,
        };
        items.push(
            SearchItem::new(
                format!("note:{}", node.id),
                SearchKind::Note,
                node.title.clone(),
                SearchAction::Note(node.id.clone()),
            )
            .subtitle(path)
            .icon(icon),
        );
    }
}

fn push_pages(items: &mut Vec<SearchItem>) {
    for (route, key, icon) in PAGES {
        items.push(
            SearchItem::new(
                format!("page:{route}"),
                SearchKind::Page,
                tr!(key),
                SearchAction::Page(route),
            )
            .icon(icon)
            .keywords(page_keywords(route)),
        );
    }
    let settings_label = tr!("nav.settings");
    for (key, icon) in SETTINGS {
        items.push(
            SearchItem::new(
                format!("settings:{key}"),
                SearchKind::Page,
                tr!(&format!("settings.tabs.{key}.title")),
                SearchAction::Settings(key),
            )
            .subtitle(tr!(&format!("settings.tabs.{key}.subtitle")))
            .hint(settings_label.clone())
            .icon(icon)
            .keywords(settings_keywords(key)),
        );
    }
}

fn push_commands(items: &mut Vec<SearchItem>) {
    let group = tr!("search.group.commands");
    for spec in COMMANDS {
        items.push(
            SearchItem::new(
                format!("cmd:{:?}", spec.command),
                SearchKind::Command,
                tr!(spec.key),
                SearchAction::Command(spec.command),
            )
            .hint(group.clone())
            .icon(spec.icon)
            .keywords(spec.keywords),
        );
    }
}

fn push_chats(items: &mut Vec<SearchItem>) {
    let ctx = use_context::<SynChatCtx>();
    // Архивные чаты в выдачу не идут: их место — Настройки → Архив.
    for meta in ctx.chats.get().iter().filter(|m| !m.archived) {
        items.push(
            SearchItem::new(
                format!("chat:{}", meta.id),
                SearchKind::Chat,
                meta.title.clone(),
                SearchAction::Chat(meta.id.clone()),
            )
            .subtitle(meta.preview.clone())
            .hint(meta.model_name.clone().unwrap_or_default())
            .icon(MI_CHAT),
        );
    }
}

fn push_messages(items: &mut Vec<SearchItem>, scan: &ScanData) {
    for m in &scan.messages {
        let hint = if m.time.is_empty() {
            m.chat_title.clone()
        } else {
            format!("{} · {}", m.chat_title, m.time)
        };
        items.push(
            SearchItem::new(
                format!("msg:{}:{}", m.chat_id, m.index),
                SearchKind::Message,
                m.text.clone(),
                SearchAction::Message {
                    chat_id: m.chat_id.clone(),
                    index: m.index,
                },
            )
            .subtitle(m.author.clone())
            .hint(hint)
            .icon(MI_EDIT_NOTE)
            // Чат находится и по своему названию: «покажи, что я писал в
            // чате про ffmpeg» — запрос из двух слов сразу по обоим полям.
            .keyword(m.chat_title.clone())
            .secondary(SearchAction::Chat(m.chat_id.clone())),
        );
    }
}

fn push_models(items: &mut Vec<SearchItem>, scan: &ScanData) {
    for m in &scan.models {
        let size = if m.bytes > 0 {
            format_size(m.bytes)
        } else {
            String::new()
        };
        items.push(
            SearchItem::new(
                format!("model:{}", m.path.display()),
                SearchKind::Model,
                m.name.clone(),
                SearchAction::Model(m.path.clone()),
            )
            .subtitle(m.folder.clone())
            .hint(size)
            .icon(MI_FOLDER_ZIP)
            .keyword(m.path.display().to_string())
            .secondary(SearchAction::LoadModelFile(m.path.clone())),
        );
    }
}

fn push_skills(items: &mut Vec<SearchItem>) {
    let app = use_context::<AppCtx>();
    let hint = tr!("settings.tabs.skills.title");
    for skill in app.skills.get().iter() {
        let Skill {
            id,
            name,
            description,
            ..
        } = skill;
        items.push(
            SearchItem::new(
                format!("skill:{id}"),
                SearchKind::Skill,
                name.clone(),
                SearchAction::Skill(id.clone()),
            )
            .subtitle(description.clone())
            .hint(hint.clone())
            .icon(MI_PSYCHOLOGY)
            .keyword(id),
        );
    }
}

fn push_tools(items: &mut Vec<SearchItem>) {
    let hint = tr!("search.group.tools");
    for tool in Tool::selectable() {
        items.push(
            SearchItem::new(
                format!("tool:{}", tool.key),
                SearchKind::Tool,
                crate::i18n::tool_label(tool),
                SearchAction::Tool(tool.key.to_string()),
            )
            .subtitle(tool.description)
            .hint(hint.clone())
            .icon(tool.icon)
            .keyword(tool.key)
            .secondary(SearchAction::ToggleTool(tool.key.to_string())),
        );
    }
}

fn push_templates(items: &mut Vec<SearchItem>, scan: &ScanData) {
    let custom_hint = tr!("search.template.custom");
    for t in &scan.templates {
        let hint = if t.builtin && !t.category_key.is_empty() {
            tr!(&format!("template.category.{}", t.category_key))
        } else {
            custom_hint.clone()
        };
        let (name, description) = if t.builtin {
            (
                syngui::i18n::try_tr(&format!("template.{}.name", t.id))
                    .unwrap_or_else(|| t.name.clone()),
                syngui::i18n::try_tr(&format!("template.{}.desc", t.id))
                    .unwrap_or_else(|| t.description.clone()),
            )
        } else {
            (t.name.clone(), t.description.clone())
        };
        items.push(
            SearchItem::new(
                format!("template:{}", t.id),
                SearchKind::Template,
                name,
                SearchAction::Template(t.id.clone()),
            )
            .subtitle(description)
            .hint(hint)
            .icon(MI_ACCOUNT_TREE),
        );
    }
}
