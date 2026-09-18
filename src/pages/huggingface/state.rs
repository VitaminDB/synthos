//! Контекст и типы данных страницы HuggingFace.
//!
//! `HuggingFaceCtx` — Copy-handle с реактивными сигналами; провайдится
//! единожды в `run_desktop` и читается компонентами через
//! `use_context::<HuggingFaceCtx>()`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use syngui::prelude::*;
use serde::Deserialize;

use crate::config::AppConfig;

// ─────────────────────────────────────────────────────────────────────────────
// Перечисления состояния UI
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortMode {
    Trending,
    MostDownloads,
    MostLikes,
    RecentlyUpdated,
}

impl SortMode {
    /// Поле API `?sort=...` для эндпоинта `/api/models`.
    /// Все четыре значения подтверждены на проде HF Hub.
    pub fn api_field(self) -> &'static str {
        match self {
            SortMode::Trending => "trendingScore",
            SortMode::MostDownloads => "downloads",
            SortMode::MostLikes => "likes",
            SortMode::RecentlyUpdated => "lastModified",
        }
    }

    pub fn label(self) -> String {
        match self {
            SortMode::Trending => tr!("hf.sort.trending"),
            SortMode::MostDownloads => tr!("hf.sort.most_downloads"),
            SortMode::MostLikes => tr!("hf.sort.most_likes"),
            SortMode::RecentlyUpdated => tr!("hf.sort.recently_updated"),
        }
    }
}

/// Вид списка файлов репозитория в правой панели.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilesViewMode {
    /// Строки на всю ширину: имя, размер, статус, действия, прогресс.
    List,
    /// Плитки с иконкой типа файла — как «значки» в файловом менеджере.
    Icons,
}

impl FilesViewMode {
    /// Значение для `AppConfig.hf_files_view`.
    pub fn as_config(self) -> &'static str {
        match self {
            FilesViewMode::List => "list",
            FilesViewMode::Icons => "icons",
        }
    }

    pub fn from_config(s: &str) -> Self {
        if s == "icons" { FilesViewMode::Icons } else { FilesViewMode::List }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListLoadState {
    Idle,
    Loading,
    Ready,
    Error,
}

// 0 — README, 1 — Files. Используется как TabState.

#[derive(Clone, Debug, PartialEq)]
pub enum DlStatus {
    /// В очереди, ждёт освобождения слота concurrent-лимита.
    Pending,
    Active,
    /// Приостановлено пользователем: `.part` сохранён, кнопка ▶ Продолжить
    /// возобновляет докачку с того же места.
    Paused,
    /// Остановлено пользователем: `.part` сохранён, кнопка ⬇ Скачать резюмит.
    /// Семантически как Paused, но отдельный статус для другого UX-акцента.
    Stopped,
    Done,
    Error(String),
}

/// Состояние SHA-256-верификации файла. Запускается автоматически после
/// успешного download_file → Done и вручную по кнопке «SHA256». Используется
/// в UI для бейджа справа от имени файла.
#[derive(Clone, Debug, PartialEq)]
pub enum VerifyStatus {
    /// Никогда не запускали. Дефолт для свежих/Pending файлов.
    Unknown,
    /// Идёт вычисление SHA-256 в фоновом потоке.
    Computing,
    /// Совпало с `expected_sha256` (или просто посчиталось, если expected отсутствует).
    /// В случае совпадения с expected — кладём `Match { sha256, expected: true }`;
    /// если expected'а нет — `Match { sha256, expected: false }` (информационно).
    Match { sha256: String, has_expected: bool },
    /// Не совпало. Хэш файла отличается от того, что обещал HF API.
    /// Файл повреждён или подменён, нужно перекачать.
    Mismatch { actual: String, expected: String },
    /// I/O / прочая ошибка при чтении файла.
    Error(String),
}

/// Прогресс одного HTTP-Range сегмента файла. `from..to` (inclusive) —
/// диапазон байт, который качает этот сегмент; `bytes_done` — сколько уже
/// записано (0..=to-from+1).
#[derive(Clone, Debug)]
pub struct SegmentProgress {
    pub from: u64,
    pub to: u64,
    pub bytes_done: u64,
}

#[derive(Clone, Debug)]
pub struct DownloadState {
    pub repo_id: String,
    pub filename: String,
    pub bytes_done: u64,
    /// `0` если сервер не отдал Content-Length.
    pub total: u64,
    pub status: DlStatus,
    pub dest_path: PathBuf,
    /// Прогресс по каждому сегменту. Пустой — single-stream (нет Range-разбиения).
    /// Заполняется в `download::start_actual_download` перед спавном задач.
    #[allow(dead_code)]
    pub segments: Vec<SegmentProgress>,
    /// Скорость в байт/сек, EMA по интервалам ~500мс. 0 — пока нечего показать.
    pub speed_bps: f64,
    /// Якорь для расчёта скорости: время и байты предыдущей выборки.
    /// Pop'ается каждые ~500мс в api.rs callbacks.
    pub speed_sample_at: Option<Instant>,
    pub speed_sample_bytes: u64,
    /// Сколько раз эту загрузку перезапустили из-за транзиентной ошибки.
    /// Сбрасывается при успехе или ручном retry. UI показывает «попытка N/3».
    pub retry_count: u32,
    /// Throttle для записи `.part.meta` в сегментированном download.
    /// Обновляется в main-thread aggregation closure после агрегата
    /// `bytes_done`. Меньше 1 с между flush'ами — экономим I/O.
    pub last_meta_flush_at: Option<Instant>,
    /// Ожидаемый SHA-256 файла из `HfSibling.lfs.sha256`. Если `None` —
    /// HF не предоставил checksum (не-LFS файл), авто-верификация не
    /// запускается; ручная кнопка «SHA256» всё равно работает и
    /// просто вычислит хэш.
    pub expected_sha256: Option<String>,
    /// Текущее состояние верификации (см. `VerifyStatus`).
    pub verify: VerifyStatus,
}

// ─────────────────────────────────────────────────────────────────────────────
// Сериализуемые типы API HuggingFace
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct HfModel {
    pub id: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub likes: u64,
    #[serde(default, rename = "lastModified")]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub pipeline_tag: Option<String>,
    #[serde(default)]
    pub library_name: Option<String>,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct HfSibling {
    pub rfilename: String,
    #[serde(default)]
    pub size: Option<u64>,
    /// LFS-метаданные. Заполняются для `safetensors`/`gguf`/больших весов
    /// (всё, что лежит в Git-LFS). `lfs.sha256` — каноничный hex digest,
    /// используется для авто-верификации после скачивания и для ручной
    /// кнопки «SHA256». Для маленьких git-blob'ов (config.json, README) поле
    /// отсутствует — таких файлов мы не верифицируем (нечего сравнивать).
    #[serde(default)]
    pub lfs: Option<HfLfs>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct HfLfs {
    /// Hex-строка SHA-256 (без `sha256:` префикса в новом API).
    /// На некоторых ответах поле называется `oid` — поддерживаем оба.
    #[serde(default, alias = "oid")]
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct HfModelDetails {
    pub id: String,
    #[serde(default)]
    pub siblings: Vec<HfSibling>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, rename = "lastModified")]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub likes: u64,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub pipeline_tag: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// HuggingFaceCtx
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub struct HuggingFaceCtx {
    /// Текст в поле поиска. Меняется на каждый keystroke (`on_change`).
    pub search_query: RwSignal<String>,
    /// Закоммиченный (по Enter / сменой сорта) запрос — то, что реально
    /// уходит в HF API. Разделение нужно, чтобы не перезапускать запросы
    /// на каждый символ.
    pub committed_query: RwSignal<String>,
    /// Текущий режим сортировки списка.
    pub sort_mode: RwSignal<SortMode>,
    /// Найденные модели (после последнего успешного запроса).
    pub models: RwSignal<Vec<HfModel>>,
    pub list_state: RwSignal<ListLoadState>,
    /// Текст ошибки последнего запроса списка (для UI). `None` — успех.
    pub list_error: RwSignal<Option<String>>,
    /// Выбранная карточка (repo_id) для отображения справа.
    pub selected_model: RwSignal<Option<String>>,
    /// Детали выбранной модели (загружаются отдельно от списка).
    pub model_details: RwSignal<Option<HfModelDetails>>,
    /// README выбранной модели. Пусто = «нет README» или ещё не загружено.
    pub readme_text: RwSignal<String>,
    pub readme_loading: RwSignal<bool>,
    /// Per-file карта скачиваний. Ключ: `"{repo_id}/{filename}"`.
    pub downloads: RwSignal<HashMap<String, DownloadState>>,
    /// Каталог кэша HF-моделей (синхронизирован с `AppConfig.hf_cache_dir`).
    pub cache_dir: RwSignal<String>,
    /// HF access-token (синхронизирован с `AppConfig.hf_token`). Нужен для
    /// gated/private моделей и снятия anon rate-limit. Изменение в Settings →
    /// HuggingFace прокидывается в `api::set_token` через эффект в `lib.rs`.
    pub token: RwSignal<String>,
    /// Диалог выбора каталога открыт?
    pub cache_dir_dialog_open: RwSignal<bool>,
    /// Очередь файлов «после выбора каталога открыть на закачку». В отличие от
    /// `download_queue` это user-intent: даже если cache-dir ещё не выбран,
    /// мы сохраняем здесь список (repo_id, filename) и после accept_default /
    /// pick_folder отдаём их в download::start_download.
    pub pending_download: RwSignal<Vec<(String, String)>>,
    /// Сколько файлов скачивается параллельно. Синхронизирован с
    /// `AppConfig.hf_concurrent_downloads`. Изменения через SpinBox toolbar'а
    /// или Settings → HuggingFace немедленно перезапускают `try_drain_queue`,
    /// чтобы при увеличении лимита pending-задачи тут же стартовали.
    pub concurrent_limit: RwSignal<u32>,
    /// Кол-во HTTP-Range сегментов на один файл. 1 — single-stream;
    /// 2..16 — параллельные диапазоны через `Range: bytes=A-B`.
    pub segments_per_file: RwSignal<u32>,
    /// Глобальный лимит скорости скачивания в МБ/с. `0` — без лимита.
    /// Синхронизирован с `AppConfig.hf_speed_limit_mbps`; изменение через тулбар
    /// или Settings зеркалится в `rate::set_limit_bps` (эффект в `lib.rs`).
    pub speed_limit_mbps: RwSignal<u32>,
    /// Глобальная пауза всех загрузок. При `true` `try_drain_queue` не стартует
    /// новые задачи, а `pause_all` переводит активные/очередь в Paused.
    pub global_paused: RwSignal<bool>,
    /// FIFO-очередь ключей `{repo_id}/{filename}` — ожидают активный слот.
    /// При завершении любого Active-скачивания вызывается `try_drain_queue`.
    pub download_queue: RwSignal<Vec<String>>,
    /// Счётчик активных (Active) загрузок. Считаем сами (не filter по map'е)
    /// чтобы не зависеть от порядка обновлений в run_on_main_thread.
    pub active_downloads: RwSignal<u32>,
    /// Тумблер «Скачать всё пропускает onnx/openvino/fp32/bin». Синхронизирован
    /// с `AppConfig.hf_skip_unwanted_formats`; переключение через тулбар Files
    /// перезаписывает config.json (autosave-эффект в `lib.rs`).
    pub skip_unwanted_formats: RwSignal<bool>,
    pub gguf_support: RwSignal<bool>,
    pub gguf_filter: RwSignal<bool>,
    pub convert_active: RwSignal<Option<String>>,
    pub convert_progress: RwSignal<f32>,
    /// Отмеченные чекбоксами файлы для выборочной закачки. Ключи —
    /// `{repo_id}/{filename}` (как в `downloads`). Сбрасывается при смене
    /// модели (`actions::select_model`). Кнопка «Скачать выбранные» enqueue'ит
    /// только эти файлы (без фильтра форматов — выбор ручной).
    pub selected_files: RwSignal<HashSet<String>>,
    /// Список или плитки в панели файлов. Синхронизирован с
    /// `AppConfig.hf_files_view`.
    pub files_view_mode: RwSignal<FilesViewMode>,
    /// Свёрнутые каталоги панели файлов, ключ `{repo_id}/{dir}`. На сессию:
    /// у другого репозитория другие каталоги, хранить на диске нечего.
    pub collapsed_dirs: RwSignal<HashSet<String>>,
    /// Нижняя панель загрузок развёрнута (очередь + настройки) или свёрнута до
    /// одной строки со сводкой. Синхронизирован с `AppConfig.hf_dock_expanded`.
    pub dock_expanded: RwSignal<bool>,
}

/// Токен на старте: поле конфига имеет приоритет; если пусто — стандартный
/// HF-резолюшн (env `HF_TOKEN`/`HUGGING_FACE_HUB_TOKEN`, затем файл
/// `~/.cache/huggingface/token` от `huggingface-cli login`). Возвращённое
/// значение подставляется в поле Settings и персистится — auto-import входа.
fn resolve_initial_token(cfg_token: &str) -> String {
    let t = cfg_token.trim();
    if !t.is_empty() {
        return t.to_string();
    }
    for var in ["HF_TOKEN", "HUGGING_FACE_HUB_TOKEN", "HUGGINGFACE_TOKEN"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(hf_home) = std::env::var("HF_HOME") {
        if !hf_home.trim().is_empty() {
            candidates.push(PathBuf::from(hf_home).join("token"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".cache/huggingface/token"));
    }
    for path in candidates {
        if let Ok(s) = std::fs::read_to_string(&path) {
            let s = s.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    String::new()
}

impl HuggingFaceCtx {
    pub fn new(cfg: &AppConfig) -> Self {
        let token0 = resolve_initial_token(&cfg.hf_token);
        // Стартовая установка токена в api-слой до первых запросов.
        super::api::set_token(&token0);
        Self {
            search_query: use_signal(String::new()),
            committed_query: use_signal(String::new()),
            sort_mode: use_signal(SortMode::Trending),
            models: use_signal(Vec::new()),
            list_state: use_signal(ListLoadState::Idle),
            list_error: use_signal(None),
            selected_model: use_signal(None),
            model_details: use_signal(None),
            readme_text: use_signal(String::new()),
            readme_loading: use_signal(false),
            downloads: use_signal(HashMap::new()),
            cache_dir: use_signal(cfg.hf_cache_dir.clone()),
            token: use_signal(token0),
            cache_dir_dialog_open: use_signal(false),
            pending_download: use_signal(Vec::new()),
            concurrent_limit: use_signal(cfg.hf_concurrent_downloads.max(1)),
            segments_per_file: use_signal(cfg.hf_segments_per_file.clamp(1, 16)),
            speed_limit_mbps: use_signal(cfg.hf_speed_limit_mbps),
            global_paused: use_signal(false),
            download_queue: use_signal(Vec::new()),
            active_downloads: use_signal(0),
            skip_unwanted_formats: use_signal(cfg.hf_skip_unwanted_formats),
            gguf_support: use_signal(cfg.hf_gguf_support),
            gguf_filter: use_signal(false),
            convert_active: use_signal(None),
            convert_progress: use_signal(0.0),
            selected_files: use_signal(HashSet::new()),
            collapsed_dirs: use_signal(HashSet::new()),
            files_view_mode: use_signal(FilesViewMode::from_config(&cfg.hf_files_view)),
            dock_expanded: use_signal(cfg.hf_dock_expanded),
        }
    }
}
