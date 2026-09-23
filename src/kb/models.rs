//! Поиск моделей KB (эмбеддер и реранкер) на диске.
//!
//! Модели живут там же, где остальные — в каталоге из «AI модели»
//! (`AppConfig.models_dir`), `.syn`-бандлами. Путь из `KbConfig` — только
//! ручное переопределение: пустой или ведущий в никуда не мешает найти
//! модель самим.
//!
//! Порядок поиска:
//! 1. путь из конфига, если он существует (файл-бандл или каталог-снапшот);
//!    тот же путь с дописанным `.syn` — старый дефолт был каталогом
//!    `~/models/bge-m3`, а упакованная модель лежит рядом как `bge-m3.syn`;
//! 2. известные имена в каталогах поиска (`bge-m3.syn`, каталог `bge-m3`);
//! 3. любой `.syn` с подходящими `purpose` и `arch` в метаданных — бандл
//!    могли переименовать.
//!
//! Шаг 3 открывает каждый бандл каталога (~10 мс на файл), поэтому весь
//! поиск зовут с рабочего потока, а не из отрисовки.

use std::path::{Path, PathBuf};

use synaptix_bundle::Bundle;

use crate::config::KbConfig;

/// Архитектура, которую умеет нативный BGE-стек synaptix.
const ARCH: &str = "xlm-roberta";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    Embedder,
    Reranker,
}

impl ModelKind {
    /// Имя модели без расширения: так называется и бандл, и каталог снапшота.
    pub fn stem(self) -> &'static str {
        match self {
            Self::Embedder => "bge-m3",
            Self::Reranker => "bge-reranker-v2-m3",
        }
    }

    /// Репозиторий на HuggingFace — для подсказки «где взять».
    pub fn hf_repo(self) -> &'static str {
        match self {
            Self::Embedder => "BAAI/bge-m3",
            Self::Reranker => "BAAI/bge-reranker-v2-m3",
        }
    }

    /// Значения `BundleMeta.purpose`, под которыми пакуют такую модель.
    fn purposes(self) -> &'static [&'static str] {
        match self {
            Self::Embedder => &["embed", "embedding", "embedder"],
            Self::Reranker => &["reranker", "rerank"],
        }
    }

    fn configured(self, cfg: &KbConfig) -> &str {
        match self {
            Self::Embedder => &cfg.embedder_model_path,
            Self::Reranker => &cfg.reranker_model_path,
        }
    }
}

/// Откуда взялся путь — UI подписывает «найдена автоматически» / «указана вручную».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoundBy {
    Config,
    Discovery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundModel {
    pub path: PathBuf,
    pub by: FoundBy,
    /// Размер бандла в байтах; у каталога-снапшота — 0 (не считаем).
    pub bytes: u64,
}

/// Снимок поиска обеих моделей. `searched` — где искали: показываем в
/// подсказке «не найдена».
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelPaths {
    pub embedder: Option<FoundModel>,
    pub reranker: Option<FoundModel>,
    pub searched: Vec<PathBuf>,
}

impl ModelPaths {
    pub fn get(&self, kind: ModelKind) -> Option<&FoundModel> {
        match kind {
            ModelKind::Embedder => self.embedder.as_ref(),
            ModelKind::Reranker => self.reranker.as_ref(),
        }
    }
}

/// Найти обе модели. `dirs` — каталоги поиска по убыванию приоритета
/// (первым — `models_dir`). Читает диск: звать с рабочего потока.
pub fn discover(cfg: &KbConfig, dirs: &[PathBuf]) -> ModelPaths {
    ModelPaths {
        embedder: find(ModelKind::Embedder, cfg, dirs),
        reranker: find(ModelKind::Reranker, cfg, dirs),
        searched: dirs.to_vec(),
    }
}

/// Найти одну модель; порядок — в шапке модуля.
pub fn find(kind: ModelKind, cfg: &KbConfig, dirs: &[PathBuf]) -> Option<FoundModel> {
    let configured = kind.configured(cfg).trim();
    if !configured.is_empty() {
        let raw = expand_home(configured);
        let with_ext = PathBuf::from(format!("{}.syn", raw.display()));
        for candidate in [raw, with_ext] {
            if is_model(&candidate) {
                return Some(found(candidate, FoundBy::Config));
            }
        }
    }
    for dir in dirs {
        for name in [format!("{}.syn", kind.stem()), kind.stem().to_string()] {
            let candidate = dir.join(name);
            if is_model(&candidate) {
                return Some(found(candidate, FoundBy::Discovery));
            }
        }
    }
    dirs.iter()
        .find_map(|dir| find_by_meta(kind, dir))
        .map(|p| found(p, FoundBy::Discovery))
}

/// Перебор бандлов каталога по метаданным — имя файла не важно.
fn find_by_meta(kind: ModelKind, dir: &Path) -> Option<PathBuf> {
    let mut bundles: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("syn") && p.is_file())
        .collect();
    // Порядок read_dir не определён — сортируем, чтобы выбор был стабильным.
    bundles.sort();
    bundles.into_iter().find(|path| {
        Bundle::open(path).is_ok_and(|b| {
            let meta = b.meta();
            meta.arch == ARCH && kind.purposes().contains(&meta.purpose.as_str())
        })
    })
}

/// Файл-бандл либо каталог-снапшот с `config.json`.
fn is_model(path: &Path) -> bool {
    if path.is_file() {
        return path.extension().and_then(|e| e.to_str()) == Some("syn");
    }
    path.is_dir() && path.join("config.json").is_file()
}

fn found(path: PathBuf, by: FoundBy) -> FoundModel {
    let bytes = std::fs::metadata(&path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .unwrap_or(0);
    FoundModel { path, by, bytes }
}

fn expand_home(raw: &str) -> PathBuf {
    match raw.strip_prefix("~/") {
        Some(rest) => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(rest)
        }
        None => PathBuf::from(raw),
    }
}

/// `tokenizer.json` модели — чанкеру нужен тот же словарь, что у эмбеддера.
pub fn read_tokenizer_json(model: &Path) -> Result<Vec<u8>, String> {
    if model.is_dir() {
        return std::fs::read(model.join("tokenizer.json")).map_err(|e| e.to_string());
    }
    let bundle = Bundle::open(model).map_err(|e| e.to_string())?;
    bundle
        .read_file("tokenizer.json")
        .map(|c| c.into_owned())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use synaptix_bundle::{BundleBuilder, FileTag};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "synthos-kb-models-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_bundle(path: &Path, purpose: &str, arch: &str) {
        BundleBuilder::new("test", "1")
            .arch(arch)
            .purpose(purpose)
            .add_file_bytes("tokenizer.json", b"{}".to_vec(), FileTag::Inference)
            .unwrap()
            .write(path)
            .unwrap();
    }

    fn cfg(embedder: &str, reranker: &str) -> KbConfig {
        KbConfig {
            embedder_model_path: embedder.into(),
            reranker_model_path: reranker.into(),
            ..KbConfig::default()
        }
    }

    #[test]
    fn stale_config_path_falls_back_to_models_dir() {
        let dir = temp_dir("stale");
        write_bundle(&dir.join("bge-m3.syn"), "embed", ARCH);
        let found = find(
            ModelKind::Embedder,
            &cfg("/nonexistent/models/bge-m3", ""),
            std::slice::from_ref(&dir),
        )
        .expect("бандл из каталога моделей");
        assert_eq!(found.path, dir.join("bge-m3.syn"));
        assert_eq!(found.by, FoundBy::Discovery);
        assert!(found.bytes > 0);
    }

    #[test]
    fn configured_dir_path_resolves_to_sibling_bundle() {
        let dir = temp_dir("sibling");
        write_bundle(&dir.join("bge-m3.syn"), "embed", ARCH);
        let configured = dir.join("bge-m3");
        let found = find(
            ModelKind::Embedder,
            &cfg(&configured.display().to_string(), ""),
            &[],
        )
        .expect("bge-m3 → bge-m3.syn");
        assert_eq!(found.by, FoundBy::Config);
        assert_eq!(found.path, dir.join("bge-m3.syn"));
    }

    #[test]
    fn renamed_bundle_found_by_meta_and_kinds_not_mixed() {
        let dir = temp_dir("meta");
        write_bundle(&dir.join("a-my-embedder.syn"), "embed", ARCH);
        write_bundle(&dir.join("b-my-reranker.syn"), "reranker", ARCH);
        // Эмбеддер чужой архитектуры BGE-стек не загрузит — не берём.
        write_bundle(&dir.join("0-qwen3-embedding.syn"), "embed", "qwen3");
        let paths = discover(&cfg("", ""), std::slice::from_ref(&dir));
        assert_eq!(paths.embedder.unwrap().path, dir.join("a-my-embedder.syn"));
        assert_eq!(paths.reranker.unwrap().path, dir.join("b-my-reranker.syn"));
    }

    #[test]
    fn nothing_found_reports_searched_dirs() {
        let dir = temp_dir("empty");
        let paths = discover(&cfg("", ""), std::slice::from_ref(&dir));
        assert!(paths.embedder.is_none() && paths.reranker.is_none());
        assert_eq!(paths.searched, vec![dir]);
    }

    #[test]
    fn tokenizer_json_read_from_bundle_and_dir() {
        let dir = temp_dir("tok");
        write_bundle(&dir.join("bge-m3.syn"), "embed", ARCH);
        assert_eq!(read_tokenizer_json(&dir.join("bge-m3.syn")).unwrap(), b"{}");
        let snap = dir.join("snap");
        std::fs::create_dir_all(&snap).unwrap();
        std::fs::write(snap.join("tokenizer.json"), b"[]").unwrap();
        assert_eq!(read_tokenizer_json(&snap).unwrap(), b"[]");
    }
}
