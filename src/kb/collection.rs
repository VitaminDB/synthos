//! Метаданные коллекции и реестр (registry) — список всех коллекций
//! на диске.
//!
//! Коллекция = одна `~/.config/synthos/kb/<id>.sqlite` БД. ID — короткий
//! UUID-подобный идентификатор (8 hex chars), генерируется при создании.
//! Имя пользователя (`name`) хранится внутри самой БД (таблица `collections`),
//! а не в имени файла — чтобы переименование не требовало fs-rename.
//!
//! При старте приложения `CollectionRegistry::scan()` итерирует
//! `kb_dir`, читает `name` из каждой БД, сортирует ASC по имени.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::store::{Store, StoreError};

/// Лёгкая копия метаданных коллекции для UI. Полная сущность хранится
/// в БД, тут — то, что нужно для боковой панели и dropdown'а.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionMeta {
    pub id: String,
    pub name: String,
    pub created_at: i64,         // unix sec
    pub embedding_model: String, // basename модели, например "bge-m3"
    pub embedding_dim: i32,
    pub chunk_target_tokens: i32,
    pub chunk_overlap_tokens: i32,
    /// Кол-во индексированных документов — для UI badge. Не сохраняется
    /// в БД; обновляется при `scan()` через `Store::stats()`.
    #[serde(default)]
    pub document_count: i64,
    #[serde(default)]
    pub chunk_count: i64,
}

impl CollectionMeta {
    /// Сгенерировать новый id: 8 hex-символов из времени и PID. Берём
    /// МЛАДШИЕ 32 бита наносекунд: старшие меняются раз в ~4,3 с, и две
    /// коллекции, созданные подряд, получали один id и один файл БД.
    /// От совпадения с уже существующим файлом страхует `create`.
    pub fn new_id() -> String {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id() as u128;
        let mix = nanos.wrapping_add(pid.wrapping_mul(2654435761));
        format!("{:08x}", mix as u32)
    }

    /// Путь к БД на диске.
    pub fn db_path(&self, kb_dir: &Path) -> PathBuf {
        kb_dir.join(format!("{}.sqlite", self.id))
    }
}

/// Реестр коллекций на диске. Не держит открытых БД — это просто
/// in-memory snapshot, который обновляется по запросу.
#[derive(Debug, Clone)]
pub struct CollectionRegistry {
    pub kb_dir: PathBuf,
    pub items: Vec<CollectionMeta>,
}

impl CollectionRegistry {
    pub fn new(kb_dir: impl Into<PathBuf>) -> Self {
        Self {
            kb_dir: kb_dir.into(),
            items: Vec::new(),
        }
    }

    /// Просканировать `kb_dir` и собрать список коллекций.
    /// Не падает на сломанных БД — пишет в лог и пропускает.
    pub fn scan(&mut self) {
        self.items.clear();
        if !self.kb_dir.is_dir() {
            // Создаём каталог при первом обращении — сценарий
            // «пользователь только что включил KB».
            if let Err(e) = std::fs::create_dir_all(&self.kb_dir) {
                log::warn!("kb_dir {} не создаётся: {e}", self.kb_dir.display());
                return;
            }
        }
        let read_dir = match std::fs::read_dir(&self.kb_dir) {
            Ok(rd) => rd,
            Err(e) => {
                log::warn!("kb_dir read_dir failed: {e}");
                return;
            }
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("sqlite") {
                continue;
            }
            match Self::read_meta(&path) {
                Ok(meta) => self.items.push(meta),
                Err(e) => {
                    log::warn!("kb: пропускаю битый {}: {e}", path.display());
                }
            }
        }
        self.items.sort_by(|a, b| a.name.cmp(&b.name));
    }

    fn read_meta(db_path: &Path) -> Result<CollectionMeta, StoreError> {
        let store = Store::open(db_path)?;
        let mut meta = store.read_meta()?;
        let stats = store.stats()?;
        meta.document_count = stats.document_count;
        meta.chunk_count = stats.chunk_count;
        Ok(meta)
    }

    /// Создать новую коллекцию: создаёт файл БД, пишет meta, добавляет
    /// в reg и возвращает.
    /// Создаёт новую коллекцию. Возвращает [`CollectionMeta`] на успех.
    /// Может быть вызвана как через `&mut self`, так и (предпочтительно)
    /// через `RwSignal::update` — тогда snapshot реестра сразу подхватит
    /// новую запись.
    pub fn create(
        &mut self,
        name: String,
        embedding_model: String,
        embedding_dim: i32,
        chunk_target_tokens: i32,
        chunk_overlap_tokens: i32,
    ) -> Result<CollectionMeta, StoreError> {
        if !self.kb_dir.is_dir() {
            std::fs::create_dir_all(&self.kb_dir).map_err(StoreError::Io)?;
        }
        let mut id = CollectionMeta::new_id();
        while self.kb_dir.join(format!("{id}.sqlite")).exists() {
            id = CollectionMeta::new_id();
        }
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let meta = CollectionMeta {
            id: id.clone(),
            name,
            created_at: now,
            embedding_model,
            embedding_dim,
            chunk_target_tokens,
            chunk_overlap_tokens,
            document_count: 0,
            chunk_count: 0,
        };
        let path = meta.db_path(&self.kb_dir);
        let mut store = Store::open(&path)?;
        store.ensure_schema()?;
        store.upsert_collection(&meta)?;
        self.items.push(meta.clone());
        self.items.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(meta)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), StoreError> {
        if let Some(pos) = self.items.iter().position(|m| m.id == id) {
            let path = self.items[pos].db_path(&self.kb_dir);
            // Удаляем все sqlite-файлы (БД + WAL/SHM, если есть).
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
            let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
            self.items.remove(pos);
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&CollectionMeta> {
        self.items.iter().find(|m| m.id == id)
    }

    /// Открыть Store коллекции по id. Требует уже выполненный scan().
    pub fn open_store(&self, id: &str) -> Result<Store, StoreError> {
        let meta = self
            .get(id)
            .ok_or_else(|| StoreError::NotFound(format!("collection {id}")))?;
        let path = meta.db_path(&self.kb_dir);
        Store::open(&path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_kb_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "synthos_kb_reg_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn new_id_is_8_hex_chars() {
        let id = CollectionMeta::new_id();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn create_scan_delete_roundtrip() {
        let dir = temp_kb_dir();
        let mut reg = CollectionRegistry::new(&dir);
        let meta = reg
            .create("Test KB".into(), "bge-m3".into(), 1024, 512, 64)
            .expect("create");
        assert_eq!(reg.items.len(), 1);
        assert_eq!(reg.items[0].name, "Test KB");
        assert_eq!(reg.items[0].embedding_dim, 1024);

        // Reopen via scan
        let mut reg2 = CollectionRegistry::new(&dir);
        reg2.scan();
        assert_eq!(reg2.items.len(), 1);
        assert_eq!(reg2.items[0].id, meta.id);
        assert_eq!(reg2.items[0].name, "Test KB");

        // Delete
        reg2.delete(&meta.id).expect("delete");
        assert!(reg2.items.is_empty());
        let mut reg3 = CollectionRegistry::new(&dir);
        reg3.scan();
        assert!(reg3.items.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_creates_kb_dir_if_missing() {
        let dir = std::env::temp_dir().join(format!(
            "synthos_kb_reg_missing_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut reg = CollectionRegistry::new(&dir);
        reg.scan();
        assert!(dir.is_dir(), "scan() должен создать kb_dir");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
