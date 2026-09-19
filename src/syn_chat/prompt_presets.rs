//! Библиотека системных промптов Syn-чата: именованные пресеты на диске —
//! заготовки для промптов чатов.
//!
//! С 19.09.2026 текст промпта принадлежит чату (`chat_settings::ChatSettings`
//! в его файле): выбор пресета копирует текст в открытый чат ([`select`]),
//! правка текста меняет только чат, а в пресет он уходит явно
//! ([`save_to_preset`]). Чат помнит, из какого пресета взят текст
//! (`SynChatCtx.prompt_active`), — по нему панель показывает, изменён ли
//! текст относительно пресета.
//!
//! Хранится отдельно от `config.json` — в
//! `~/.config/synthos/syn_system_prompts.json`: промпты бывают на десятки
//! килобайт, и переписывать их вместе с конфигом (сплиты, профили моделей,
//! рейл) незачем. Каждая операция библиотеки пишет файл сразу
//! ([`save_now`]).
//!
//! Инвариант: в библиотеке всегда хотя бы один пресет и `active` указывает
//! на существующий. Его поддерживает [`PresetStore::normalize`] — при
//! загрузке, после удаления и при любом расхождении. `active` — пресет,
//! выбранный последним; его текст получает промпт, пока не открыт ни один
//! чат (и получили чаты, записанные до переезда промпта в чат).
//!
//! Миграция: если файла библиотеки нет, а в `AppConfig.syn_chat_system_prompt`
//! (прежнее одиночное поле) есть текст — он становится первым пресетом.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use syngui::async_runtime::spawn;
use syngui::prelude::*;

use crate::syn_chat::SynChatCtx;

/// Один именованный системный промпт.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptPreset {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

impl PromptPreset {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        let now = unix_secs();
        Self {
            id: new_id(),
            name: name.into(),
            text: text.into(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Содержимое файла библиотеки.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetStore {
    /// id активного пресета. `None` только у пустого/битого файла — после
    /// [`normalize`](Self::normalize) всегда `Some`.
    #[serde(default)]
    pub active: Option<String>,
    #[serde(default)]
    pub presets: Vec<PromptPreset>,
}

impl PresetStore {
    /// Восстанавливает инвариант «есть пресет, активный существует».
    /// `default_name` — имя пресета, который создаётся, если список пуст.
    pub fn normalize(&mut self, default_name: &str, seed_text: &str) {
        if self.presets.is_empty() {
            self.presets.push(PromptPreset::new(default_name, seed_text));
        }
        let active_ok = self
            .active
            .as_ref()
            .is_some_and(|id| self.presets.iter().any(|p| &p.id == id));
        if !active_ok {
            self.active = Some(self.presets[0].id.clone());
        }
    }

    pub fn active_preset(&self) -> Option<&PromptPreset> {
        let id = self.active.as_ref()?;
        self.presets.iter().find(|p| &p.id == id)
    }

    pub fn active_text(&self) -> String {
        self.active_preset().map(|p| p.text.clone()).unwrap_or_default()
    }
}

// ─────────────────────────── диск ───────────────────────────

pub fn store_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/synthos/syn_system_prompts.json")
}

/// Загружает библиотеку с диска. Файла нет — переносит текст из старого
/// одиночного поля конфига (`legacy_text`) и сразу сохраняет результат.
pub fn load() -> PresetStore {
    let legacy = crate::config::AppConfig::load().syn_chat_system_prompt;
    let (store, migrated) = load_from(&store_path(), &legacy, &default_name());
    if migrated {
        save_to(&store_path(), &store);
    }
    store
}

/// Ядро [`load`] без обращения к конфигу — для тестов. Возвращает
/// библиотеку и флаг «файла не было, нужно записать».
pub fn load_from(path: &std::path::Path, legacy_text: &str, default_name: &str) -> (PresetStore, bool) {
    let mut store = match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<PresetStore>(&content) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "[synthos/syn_chat] не удалось разобрать {:?}: {e}. Библиотека промптов начата заново \
                     (файл не перезаписан до первой правки).",
                    path
                );
                PresetStore::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut s = PresetStore::default();
            s.normalize(default_name, legacy_text);
            return (s, true);
        }
        Err(e) => {
            eprintln!("[synthos/syn_chat] не удалось прочитать {:?}: {e}", path);
            PresetStore::default()
        }
    };
    store.normalize(default_name, "");
    (store, false)
}

pub fn save_to(path: &std::path::Path, store: &PresetStore) {
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("[synthos/syn_chat] не удалось создать {:?}: {e}", dir);
            return;
        }
    }
    match serde_json::to_string_pretty(store) {
        Ok(content) => {
            // Через временный файл: обрыв записи посреди большого промпта
            // не должен оставлять полфайла.
            let tmp = path.with_extension("json.tmp");
            if let Err(e) = std::fs::write(&tmp, content) {
                eprintln!("[synthos/syn_chat] не удалось записать {:?}: {e}", tmp);
                return;
            }
            if let Err(e) = std::fs::rename(&tmp, path) {
                eprintln!("[synthos/syn_chat] не удалось заменить {:?}: {e}", path);
            }
        }
        Err(e) => eprintln!("[synthos/syn_chat] ошибка сериализации библиотеки промптов: {e}"),
    }
}

/// Снимок библиотеки из сигналов контекста. Последним выбранным считается
/// пресет открытого чата; если текст чата свой — первый (так же рассудил бы
/// `normalize` при загрузке).
fn snapshot(ctx: &SynChatCtx) -> PresetStore {
    let mut store = PresetStore {
        active: Some(ctx.prompt_active.get_untracked()),
        presets: ctx.prompt_presets.get_untracked(),
    };
    store.normalize(&default_name(), "");
    store
}

/// Пишет библиотеку на диск (в фоне). Снимок, который обогнала следующая
/// запись, пропускается — иначе он лёг бы поверх более нового.
pub fn save_now(ctx: &SynChatCtx) {
    let gen = SAVE_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    let store = snapshot(ctx);
    spawn(async move {
        if SAVE_GEN.load(Ordering::Relaxed) == gen {
            save_to(&store_path(), &store);
        }
    });
}

static SAVE_GEN: AtomicU64 = AtomicU64::new(0);

// ─────────────────────────── операции из UI ───────────────────────────

/// Пресет, из которого взят текст открытого чата, если он ещё в библиотеке.
pub fn origin(ctx: &SynChatCtx) -> Option<PromptPreset> {
    let id = ctx.prompt_active.get_untracked();
    ctx.prompt_presets
        .get_untracked()
        .into_iter()
        .find(|p| p.id == id)
}

/// Текст чата разошёлся с пресетом, из которого взят.
pub fn is_modified(ctx: &SynChatCtx) -> bool {
    origin(ctx).is_some_and(|p| ctx.system_prompt.with_untracked(|t| *t != p.text))
}

/// Выбор пресета в дропдауне. Если текст чата пропал бы — он изменён или
/// не из библиотеки вовсе, — сначала спрашиваем; иначе копируем сразу.
pub fn request_select(ctx: &SynChatCtx, id: &str) {
    let Some(target) = ctx.prompt_presets.get_untracked().into_iter().find(|p| p.id == id) else {
        return;
    };
    let text = ctx.system_prompt.get_untracked();
    // Терять нечего: текст пуст, совпадает со своим пресетом или с выбранным.
    let nothing_lost = text.trim().is_empty()
        || text == target.text
        || origin(ctx).is_some_and(|p| p.text == text);
    if nothing_lost {
        select(ctx, id);
    } else {
        ctx.prompt_dialog.set(Some(PromptDialog::Replace {
            id: target.id,
            name: target.name,
        }));
    }
}

/// Копирует текст пресета в промпт открытого чата: он уходит модели со
/// следующего хода и сохраняется в файле чата. Повторный выбор того же
/// пресета возвращает его текст.
pub fn select(ctx: &SynChatCtx, id: &str) {
    let Some(text) = ctx
        .prompt_presets
        .get_untracked()
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.text)
    else {
        return;
    };
    ctx.prompt_active.set(id.to_string());
    ctx.system_prompt.set(text);
    save_now(ctx);
}

/// Записывает текст чата в пресет, из которого он взят.
pub fn save_to_preset(ctx: &SynChatCtx) {
    let id = ctx.prompt_active.get_untracked();
    let text = ctx.system_prompt.get_untracked();
    let mut changed = false;
    ctx.prompt_presets.update(|list| {
        if let Some(p) = list.iter_mut().find(|p| p.id == id) {
            if p.text != text {
                p.text = text.clone();
                p.updated_at = unix_secs();
                changed = true;
            }
        }
    });
    if changed {
        save_now(ctx);
    }
}

/// Создаёт пресет, и чат берёт промпт из него. `copy_current` — пресет
/// получает текст чата (чат его не теряет), иначе пресет и промпт чата
/// начинаются с пустого.
pub fn create(ctx: &SynChatCtx, name: &str, copy_current: bool) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let text = if copy_current {
        ctx.system_prompt.get_untracked()
    } else {
        String::new()
    };
    let preset = PromptPreset::new(name, text.clone());
    let id = preset.id.clone();
    ctx.prompt_presets.update(|list| list.push(preset));
    ctx.prompt_active.set(id);
    ctx.system_prompt.set(text);
    save_now(ctx);
}

pub fn rename(ctx: &SynChatCtx, id: &str, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let mut changed = false;
    ctx.prompt_presets.update(|list| {
        if let Some(p) = list.iter_mut().find(|p| p.id == id) {
            if p.name != name {
                p.name = name.to_string();
                p.updated_at = unix_secs();
                changed = true;
            }
        }
    });
    if changed {
        save_now(ctx);
    }
}

/// Удаляет пресет из библиотеки. Промпты чатов — их собственные копии и
/// остаются как были; у открытого чата, взявшего текст из этого пресета,
/// текст становится своим. Если пресет был последним, библиотека получает
/// новый пустой пресет по умолчанию.
pub fn delete(ctx: &SynChatCtx, id: &str) {
    let mut list = ctx.prompt_presets.get_untracked();
    let Some(pos) = list.iter().position(|p| p.id == id) else {
        return;
    };
    list.remove(pos);
    let mut store = PresetStore {
        active: Some(ctx.prompt_active.get_untracked()),
        presets: list,
    };
    store.normalize(&default_name(), "");
    ctx.prompt_presets.set(store.presets);
    if ctx.prompt_active.get_untracked() == id {
        ctx.prompt_active.set(String::new());
    }
    save_now(ctx);
}

/// Имя для нового пресета: «Промпт 2», «Промпт 3»… — первое свободное.
pub fn suggest_name(ctx: &SynChatCtx) -> String {
    let taken: Vec<String> = ctx.prompt_presets.get_untracked().into_iter().map(|p| p.name).collect();
    for n in 1..=999u32 {
        let candidate = tr!("chat.right.system.preset.suggested_name", n = n);
        if !taken.iter().any(|t| t == &candidate) {
            return candidate;
        }
    }
    tr!("chat.right.system.preset.suggested_name", n = taken.len() + 1)
}

pub fn default_name() -> String {
    tr!("chat.right.system.preset.default_name")
}

/// Что показывает модальный диалог над панелью «Система».
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptDialog {
    Create,
    Rename { id: String, name: String },
    Delete { id: String, name: String },
    /// Заменить изменённый текст чата текстом пресета `id`.
    Replace { id: String, name: String },
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // Счётчик — на случай двух пресетов в один тик часов.
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:016x}{n:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("synthos-prompt-presets-{}-{}", tag, new_id()));
        dir.join("syn_system_prompts.json")
    }

    #[test]
    fn missing_file_migrates_legacy_text_into_first_preset() {
        let path = temp_path("migrate");
        let (store, needs_write) = load_from(&path, "Ты — помощник", "По умолчанию");
        assert!(needs_write);
        assert_eq!(store.presets.len(), 1);
        assert_eq!(store.presets[0].name, "По умолчанию");
        assert_eq!(store.presets[0].text, "Ты — помощник");
        assert_eq!(store.active.as_deref(), Some(store.presets[0].id.as_str()));
    }

    #[test]
    fn roundtrip_keeps_presets_and_active() {
        let path = temp_path("roundtrip");
        let a = PromptPreset::new("A", "text a");
        let b = PromptPreset::new("B", "text b");
        let store = PresetStore {
            active: Some(b.id.clone()),
            presets: vec![a, b],
        };
        save_to(&path, &store);
        let (loaded, needs_write) = load_from(&path, "ignored legacy", "Default");
        assert!(!needs_write);
        assert_eq!(loaded, store);
        assert_eq!(loaded.active_text(), "text b");
        assert!(!path.with_extension("json.tmp").exists(), "временный файл должен быть переименован");
    }

    #[test]
    fn normalize_repairs_dangling_active_and_empty_list() {
        let mut empty = PresetStore::default();
        empty.normalize("Default", "seed");
        assert_eq!(empty.presets.len(), 1);
        assert_eq!(empty.presets[0].text, "seed");
        assert_eq!(empty.active, Some(empty.presets[0].id.clone()));

        let p = PromptPreset::new("Only", "");
        let mut dangling = PresetStore {
            active: Some("missing".into()),
            presets: vec![p.clone()],
        };
        dangling.normalize("Default", "");
        assert_eq!(dangling.active.as_deref(), Some(p.id.as_str()));
        assert_eq!(dangling.presets.len(), 1);
    }

    #[test]
    fn broken_file_starts_fresh_without_legacy_text() {
        let path = temp_path("broken");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        let (store, needs_write) = load_from(&path, "legacy", "Default");
        assert!(!needs_write, "битый файл не перезаписываем молча");
        assert_eq!(store.presets.len(), 1);
        assert_eq!(store.presets[0].text, "");
    }

    #[test]
    fn ids_are_unique_within_one_tick() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
    }
}
