//! Реактивное состояние режима «Заметки».
//!
//! Модель «список документов + активный» (как SynChatCtx): дерево vault'а —
//! снимок скана диска, открытые страницы — плитки рейла (`RailEntry::Note`),
//! у каждой — своя [`DocumentEditorHandle`] (общая модель редактора и сигнал
//! ревизии; на нём в T2 повиснет автосейв).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use syngui::prelude::*;
use syngui::widgets::input::document_editor::DocumentEditorHandle;

use crate::config::{now_millis, AppConfig, NotesOpenState};

use super::storage::{self, VaultEntry, VaultEntryKind};

/// Тип открытой плитки заметок.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    Page,
    Base,
    Canvas,
}

/// Открытая страница (плитка рейла).
#[derive(Clone)]
pub struct OpenNote {
    /// Vault-относительный путь с расширением — стабильный ключ.
    pub path: String,
    pub kind: NoteKind,
    pub title: String,
    /// Unix-миллисекунды открытия — порядок в рейле.
    pub opened_at: u64,
    /// Исходник файла на момент открытия/последней перезагрузки.
    pub source: Arc<String>,
    /// Общая модель редактора: сериализация + сигнал ревизии.
    pub handle: DocumentEditorHandle,
}

#[derive(Clone, Copy)]
pub struct NotesCtx {
    /// Абсолютный путь vault'а.
    pub vault_path: RwSignal<PathBuf>,
    /// Плоское дерево vault'а (DFS с глубинами).
    pub tree: RwSignal<Vec<VaultEntry>>,
    /// Свёрнутые папки (vault-относительные пути).
    pub collapsed: RwSignal<HashSet<String>>,
    pub open: RwSignal<Vec<OpenNote>>,
    /// Путь активной плитки.
    pub active: RwSignal<Option<String>>,
    /// Активная вкладка правой панели: 0 — Вставка, 1 — Свойства, 2 — Связи.
    pub right_tab: RwSignal<usize>,
}

impl NotesCtx {
    /// Начальное состояние: скан vault'а + восстановление открытых плиток.
    pub fn new_or_restore(cfg: &AppConfig) -> Self {
        let root = storage::resolve_vault_path(&cfg.notes_vault_path);
        storage::ensure_vault(&root);
        let tree = storage::scan(&root);

        let mut open: Vec<OpenNote> = Vec::new();
        for st in &cfg.notes_open {
            if let Some(note) = load_note(&root, &st.path, st.opened_at) {
                open.push(note);
            }
        }
        let active = cfg
            .notes_active
            .clone()
            .filter(|p| open.iter().any(|n| &n.path == p))
            .or_else(|| open.last().map(|n| n.path.clone()));

        Self {
            vault_path: use_signal(root),
            tree: use_signal(tree),
            collapsed: use_signal(HashSet::new()),
            open: use_signal(open),
            active: use_signal(active),
            right_tab: use_signal(0),
        }
    }

    /// Перечитать дерево с диска.
    pub fn rescan(&self) {
        let root = self.vault_path.get_untracked();
        self.tree.set(storage::scan(&root));
    }

    /// Открыть страницу (или активировать уже открытую) и перейти в режим.
    pub fn open_path(&self, rel: &str) {
        let already = self
            .open
            .get_untracked()
            .iter()
            .any(|n| n.path == rel);
        if !already {
            let root = self.vault_path.get_untracked();
            let Some(note) = load_note(&root, rel, now_millis()) else {
                log::warn!("notes: не удалось открыть {rel}");
                return;
            };
            self.open.update(|v| v.push(note));
        }
        self.active.set(Some(rel.to_string()));
    }

    pub fn activate(&self, rel: &str) {
        if self.open.get_untracked().iter().any(|n| n.path == rel) {
            self.active.set(Some(rel.to_string()));
        }
    }

    /// Закрыть плитку (файл остаётся). Активной становится соседняя.
    pub fn close(&self, rel: &str) {
        let mut next_active: Option<String> = None;
        self.open.update(|v| {
            if let Some(idx) = v.iter().position(|n| n.path == rel) {
                v.remove(idx);
                next_active = v
                    .get(idx.saturating_sub(1))
                    .or_else(|| v.last())
                    .map(|n| n.path.clone());
            } else {
                next_active = v.last().map(|n| n.path.clone());
            }
        });
        if self.active.get_untracked().as_deref() == Some(rel) {
            self.active.set(next_active);
        }
    }

    /// Активная открытая заметка.
    pub fn active_note(&self) -> Option<OpenNote> {
        let active = self.active.get()?;
        self.open.get().into_iter().find(|n| n.path == active)
    }

    /// Создать страницу в корне vault'а и открыть её.
    pub fn create_page(&self, base_title: &str) {
        let root = self.vault_path.get_untracked();
        match storage::create_page(&root, base_title) {
            Ok(rel) => {
                self.rescan();
                self.open_path(&rel);
            }
            Err(e) => log::warn!("notes: не удалось создать страницу: {e}"),
        }
    }

    /// Удалить файл/папку с диска, закрыв связанные плитки.
    pub fn delete_entry(&self, rel: &str) {
        let root = self.vault_path.get_untracked();
        if let Err(e) = storage::delete(&root, rel) {
            log::warn!("notes: не удалось удалить {rel}: {e}");
            return;
        }
        // Закрываем плитки удалённого файла и всего поддерева папки.
        let prefix = format!("{rel}/");
        let doomed: Vec<String> = self
            .open
            .get_untracked()
            .iter()
            .filter(|n| n.path == rel || n.path.starts_with(&prefix))
            .map(|n| n.path.clone())
            .collect();
        for p in doomed {
            self.close(&p);
        }
        self.rescan();
    }

    /// Снимок для автосейва конфига.
    pub fn open_state(&self) -> (Vec<NotesOpenState>, Option<String>) {
        let open = self
            .open
            .get()
            .iter()
            .map(|n| NotesOpenState { path: n.path.clone(), opened_at: n.opened_at })
            .collect();
        (open, self.active.get())
    }
}

/// Загрузка файла в OpenNote. Базы/канвасы пока открываются как плитки
/// с заглушкой (редакторы приходят этапами T5–T8).
fn load_note(root: &std::path::Path, rel: &str, opened_at: u64) -> Option<OpenNote> {
    let kind = match storage::kind_of(rel)? {
        VaultEntryKind::Page => NoteKind::Page,
        VaultEntryKind::Base => NoteKind::Base,
        VaultEntryKind::Canvas => NoteKind::Canvas,
        VaultEntryKind::Dir => return None,
    };
    let source = storage::load(root, rel).ok()?;
    let handle = DocumentEditorHandle::new();
    Some(OpenNote {
        path: rel.to_string(),
        kind,
        title: storage::title_of(rel),
        opened_at: if opened_at == 0 { now_millis() } else { opened_at },
        source: Arc::new(source),
        handle,
    })
}
