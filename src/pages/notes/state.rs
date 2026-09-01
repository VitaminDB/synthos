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

use super::base::model::BaseDoc;
use super::base::BaseHandle;
use super::canvas::model::CanvasDoc;
use super::canvas::CanvasHandle;
use super::index::VaultIndex;
use super::storage::{self, VaultEntry, VaultEntryKind};

/// Тип открытой плитки заметок.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    Page,
    Base,
    Canvas,
}

/// Содержимое открытой плитки по типу файла.
#[derive(Clone)]
pub enum NotePayload {
    /// Текстовая страница: markdown-исходник + модель редактора.
    Page {
        source: Arc<String>,
        handle: DocumentEditorHandle,
    },
    /// База данных (`*.base.json`).
    Base(BaseHandle),
    /// Канвас (`*.canvas.json`).
    Canvas(CanvasHandle),
    /// Файл известного типа, для которого редактора ещё нет (канвас до T8)
    /// либо не распарсившийся — держим сырым, чтобы не затереть данные.
    Raw,
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
    pub payload: NotePayload,
    /// Файл изменён снаружи при несохранённых правках — показать баннер.
    pub conflict: RwSignal<bool>,
}

impl OpenNote {
    pub fn page_handle(&self) -> Option<&DocumentEditorHandle> {
        match &self.payload {
            NotePayload::Page { handle, .. } => Some(handle),
            _ => None,
        }
    }

    pub fn base_handle(&self) -> Option<&BaseHandle> {
        match &self.payload {
            NotePayload::Base(h) => Some(h),
            _ => None,
        }
    }

    pub fn canvas_handle(&self) -> Option<&CanvasHandle> {
        match &self.payload {
            NotePayload::Canvas(h) => Some(h),
            _ => None,
        }
    }

    /// Текущая ревизия правок содержимого.
    pub fn revision(&self) -> u64 {
        match &self.payload {
            NotePayload::Page { handle, .. } => handle.revision().get_untracked(),
            NotePayload::Base(h) => h.revision.get_untracked(),
            NotePayload::Canvas(h) => h.revision.get_untracked(),
            NotePayload::Raw => 0,
        }
    }

    /// Сериализация содержимого для записи на диск.
    pub fn serialize(&self) -> Option<String> {
        match &self.payload {
            NotePayload::Page { handle, .. } => Some(handle.serialize()),
            NotePayload::Base(h) => Some(h.serialize()),
            NotePayload::Canvas(h) => Some(h.serialize()),
            NotePayload::Raw => None,
        }
    }
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
    /// Индекс wiki-связей vault'а (обновляется при сохранениях и скане).
    pub index: RwSignal<Arc<VaultIndex>>,
    /// Тик перестройки блоков после patch_media (ingest вложений).
    pub media_epoch: RwSignal<u64>,
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

        let index = VaultIndex::build(&root);
        Self {
            vault_path: use_signal(root),
            tree: use_signal(tree),
            collapsed: use_signal(HashSet::new()),
            open: use_signal(open),
            active: use_signal(active),
            right_tab: use_signal(0),
            index: use_signal(Arc::new(index)),
            media_epoch: use_signal(0),
        }
    }

    /// Полная переиндексация связей (структурные изменения vault'а).
    pub fn reindex_all(&self) {
        let root = self.vault_path.get_untracked();
        self.index.set(Arc::new(VaultIndex::build(&root)));
    }

    /// Инкрементальная переиндексация одной страницы (после сохранения).
    pub fn reindex_page(&self, rel: &str, content: &str) {
        let mut idx = (*self.index.get_untracked()).clone();
        idx.update_page(rel, content);
        self.index.set(Arc::new(idx));
    }

    /// Перечитать дерево с диска.
    pub fn rescan(&self) {
        let root = self.vault_path.get_untracked();
        self.tree.set(storage::scan(&root));
        self.reindex_all();
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
        // Хвост дебаунса — на диск, историю сохранённых ревизий — забыть.
        super::autosave::flush_now(rel);
        super::autosave::forget(rel);
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

    /// Создать канвас в корне vault'а и открыть его.
    pub fn create_canvas(&self, base_title: &str) {
        let root = self.vault_path.get_untracked();
        let content = CanvasDoc::template().serialize();
        match storage::create_file(&root, base_title, ".canvas.json", &content) {
            Ok(rel) => {
                self.rescan();
                self.open_path(&rel);
            }
            Err(e) => log::warn!("notes: не удалось создать канвас: {e}"),
        }
    }

    /// Создать базу данных в корне vault'а и открыть её.
    pub fn create_base(&self, base_title: &str) {
        let root = self.vault_path.get_untracked();
        let content = BaseDoc::template().serialize();
        match storage::create_file(&root, base_title, ".base.json", &content) {
            Ok(rel) => {
                self.rescan();
                self.open_path(&rel);
            }
            Err(e) => log::warn!("notes: не удалось создать базу: {e}"),
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
            super::autosave::forget(&p);
            self.close(&p);
        }
        self.rescan();
    }

    /// Внешнее изменение файла открытой страницы (из watcher'а).
    pub fn apply_external_change(&self, rel: &str, new_content: String) {
        let Some(note) = self.open.get_untracked().into_iter().find(|n| n.path == rel) else {
            return;
        };
        let rev = note.revision();
        let unsaved = rev > super::autosave::saved_rev(rel);
        if unsaved {
            // Локальные правки против внешних — решает пользователь.
            note.conflict.set(true);
            return;
        }
        // Семантический no-op (наша же сериализация доехала с опозданием)
        // не перегружаем — иначе прыгала бы каретка.
        if note.serialize().as_deref() == Some(new_content.as_str()) {
            super::autosave::mark_saved(rel, rev);
            return;
        }
        self.reload_note(rel, new_content);
    }

    /// Перечитать страницу с диска, отбросив локальные правки
    /// (кнопка «Перечитать» в конфликте / тихая перезагрузка).
    pub fn reload_from_disk(&self, rel: &str) {
        let root = self.vault_path.get_untracked();
        match storage::load(&root, rel) {
            Ok(content) => self.reload_note(rel, content),
            Err(e) => log::warn!("notes: не удалось перечитать {rel}: {e}"),
        }
    }

    fn reload_note(&self, rel: &str, content: String) {
        let mut rev_after = 0u64;
        self.open.update(|v| {
            if let Some(n) = v.iter_mut().find(|n| n.path == rel) {
                n.conflict.set(false);
                match &mut n.payload {
                    NotePayload::Page { source, handle } => {
                        // Reparse по fingerprint нового исходника.
                        *source = Arc::new(content.clone());
                        rev_after = handle.revision().get_untracked();
                    }
                    NotePayload::Base(h) => match BaseDoc::parse(&content) {
                        Ok(doc) => {
                            h.replace(doc);
                            rev_after = h.revision.get_untracked();
                        }
                        Err(e) => log::warn!("notes: перечитка {rel} не удалась: {e}"),
                    },
                    NotePayload::Canvas(h) => match CanvasDoc::parse(&content) {
                        Ok(doc) => {
                            h.replace(doc);
                            rev_after = h.revision.get_untracked();
                        }
                        Err(e) => log::warn!("notes: перечитка {rel} не удалась: {e}"),
                    },
                    NotePayload::Raw => {}
                }
            }
        });
        super::autosave::mark_saved(rel, rev_after);
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
    let payload = match kind {
        NoteKind::Page => NotePayload::Page {
            source: Arc::new(source),
            handle: DocumentEditorHandle::new(),
        },
        NoteKind::Base => match BaseDoc::parse(&source) {
            Ok(doc) => NotePayload::Base(BaseHandle::new(doc)),
            Err(e) => {
                // Битый JSON держим сырым — не затираем данные автосейвом.
                log::warn!("notes: {rel} не распарсился как база: {e}");
                NotePayload::Raw
            }
        },
        NoteKind::Canvas => match CanvasDoc::parse(&source) {
            Ok(doc) => NotePayload::Canvas(CanvasHandle::new(doc)),
            Err(e) => {
                log::warn!("notes: {rel} не распарсился как канвас: {e}");
                NotePayload::Raw
            }
        },
    };
    Some(OpenNote {
        path: rel.to_string(),
        kind,
        title: storage::title_of(rel),
        opened_at: if opened_at == 0 { now_millis() } else { opened_at },
        payload,
        conflict: use_signal(false),
    })
}
