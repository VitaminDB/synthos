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
//! 3. модель того же вида (эмбеддер, реранкер) архитектуры XLM-RoBERTa из
//!    библиотеки службы synaptixd — бандл могли переименовать или положить в
//!    другую библиотеку.
//!
//! Поиск читает диск и спрашивает службу, поэтому его зовут с рабочего
//! потока, а не из отрисовки.

use std::path::{Path, PathBuf};

use crate::config::KbConfig;

/// Архитектура моделей BGE (эмбеддер и реранкер базы знаний).
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

    /// Вид модели в библиотеке службы.
    fn service_kind(self) -> synaptixd::ModelKind {
        match self {
            Self::Embedder => synaptixd::ModelKind::Embed,
            Self::Reranker => synaptixd::ModelKind::Rerank,
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

/// Модель библиотеки службы: (вид, архитектура, файл).
pub type ServiceModel = (synaptixd::ModelKind, String, PathBuf);

/// Модели базы знаний в библиотеке службы (пусто — служба не отвечает).
fn service_models() -> Vec<ServiceModel> {
    let Ok(list) = synaptixd::Client::connect().and_then(|mut c| c.list()) else { return Vec::new() };
    list.into_iter()
        .filter(|m| matches!(m.kind, synaptixd::ModelKind::Embed | synaptixd::ModelKind::Rerank) && !m.path.is_empty())
        .map(|m| (m.kind, m.arch, PathBuf::from(m.path)))
        .collect()
}

/// Найти обе модели. `dirs` — каталоги поиска по убыванию приоритета
/// (первым — `models_dir`). Читает диск и спрашивает службу: звать с
/// рабочего потока.
pub fn discover(cfg: &KbConfig, dirs: &[PathBuf]) -> ModelPaths {
    let service = service_models();
    ModelPaths {
        embedder: find_with(ModelKind::Embedder, cfg, dirs, &service),
        reranker: find_with(ModelKind::Reranker, cfg, dirs, &service),
        searched: dirs.to_vec(),
    }
}

/// Найти одну модель; порядок — в шапке модуля.
pub fn find(kind: ModelKind, cfg: &KbConfig, dirs: &[PathBuf]) -> Option<FoundModel> {
    find_with(kind, cfg, dirs, &service_models())
}

fn find_with(kind: ModelKind, cfg: &KbConfig, dirs: &[PathBuf], service: &[ServiceModel]) -> Option<FoundModel> {
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
    // Библиотека службы: модель того же вида и архитектуры BGE.
    let mut of_kind: Vec<&PathBuf> = service
        .iter()
        .filter(|(k, arch, p)| *k == kind.service_kind() && arch.replace('_', "-").eq_ignore_ascii_case(ARCH) && is_model(p))
        .map(|(_, _, p)| p)
        .collect();
    of_kind.sort();
    of_kind.first().map(|p| found((*p).clone(), FoundBy::Discovery))
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

#[cfg(test)]
mod tests {
    use super::*;

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

    fn write_bundle(path: &Path) {
        std::fs::write(path, b"syn-test").unwrap();
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
        write_bundle(&dir.join("bge-m3.syn"));
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
        write_bundle(&dir.join("bge-m3.syn"));
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
    fn renamed_bundle_found_in_service_and_kinds_not_mixed() {
        let dir = temp_dir("meta");
        let (emb, rer, other) = (dir.join("a-my-embedder.syn"), dir.join("b-my-reranker.syn"), dir.join("0-qwen3-embedding.syn"));
        for p in [&emb, &rer, &other] {
            write_bundle(p);
        }
        let service = vec![
            // Эмбеддер чужой архитектуры BGE-стек не загрузит — не берём.
            (synaptixd::ModelKind::Embed, "qwen3".to_string(), other.clone()),
            (synaptixd::ModelKind::Embed, "xlm-roberta".to_string(), emb.clone()),
            (synaptixd::ModelKind::Rerank, "xlm_roberta".to_string(), rer.clone()),
        ];
        let c = cfg("", "");
        assert_eq!(find_with(ModelKind::Embedder, &c, &[], &service).unwrap().path, emb);
        assert_eq!(find_with(ModelKind::Reranker, &c, &[], &service).unwrap().path, rer);
    }

    #[test]
    fn nothing_found_reports_searched_dirs() {
        let dir = temp_dir("empty");
        let c = cfg("", "");
        assert!(find_with(ModelKind::Embedder, &c, std::slice::from_ref(&dir), &[]).is_none());
        assert!(find_with(ModelKind::Reranker, &c, std::slice::from_ref(&dir), &[]).is_none());
    }
}
