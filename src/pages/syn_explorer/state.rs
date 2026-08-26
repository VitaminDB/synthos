//! Реактивное состояние страницы SynExplorer.
//!
//! Архитектура подражает паттерну `code_editor::state`:
//! - [`SynExplorerCtx`] — Copy-handle менеджер (закладки, активный пакет, ratio'ы).
//! - [`OpenBundle`] — Copy-handle одного открытого пакета (mmap-backed bundle
//!   живёт в процессном registry `bundle_io::registry`, здесь — реактивный
//!   снапшот для UI).
//!
//! Persist: `bookmarks`, `*_split_ratio` сохраняются в
//! `AppConfig.syn_explorer_*` через общий effect в `lib.rs` (`install_config_autosave`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use syngui::prelude::*;
use syngui::widgets::TreeNode;
use synaptix_bundle::{BundleMeta, ChunkType, FileTag};

use crate::config::AppConfig;

/// Активный таб в центре страницы.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    Overview,
    Files,
    Metadata,
    Preview,
}

impl TabKind {
    pub fn label(self) -> String {
        match self {
            TabKind::Overview => tr!("explorer.tab.overview"),
            TabKind::Files => tr!("explorer.tab.files"),
            TabKind::Metadata => tr!("explorer.tab.metadata"),
            TabKind::Preview => tr!("explorer.tab.preview"),
        }
    }
}

/// Состояние асинхронного IO. Loading/Saving блокирует часть тулбара
/// и показывает overlay-индикатор; Error пробрасывается в Error-диалог
/// и затем сбрасывается в Idle. Прогресс сжатия в фазе `Creating` лежит
/// отдельно в `SynExplorerCtx.create_progress` — он эмитится из worker'а
/// часто и менять `LoadState` каждый раз было бы дорого.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadState {
    Idle,
    Loading,
    Saving,
    Creating,
}

/// Слепок прогресса сжатия `.syn`-пакета. Один Arc<Mutex> разделяется между
/// worker-thread'ом (мутирует) и UI (читает в `Reactive`-блоке).
///
/// `RwSignal<Arc<Mutex<CreateProgress>>>` был бы идиоматичнее, но
/// `RwSignal::set` дёргает request_redraw для каждого Bytes-события, а их
/// сотни в секунду на крупном пакете. Поэтому: payload-данные живут в
/// Arc<Mutex>, а `version: RwSignal<u64>` инкрементится throttled — UI
/// перечитывает Mutex при изменении версии.
#[derive(Clone, Debug, Default)]
pub struct CreateProgress {
    /// Текущий этап (имя файла/чанка).
    pub current_item: String,
    /// Индекс текущего этапа (1-based для отображения).
    pub items_done: usize,
    /// Всего этапов в плане.
    pub items_total: usize,
    /// Скопировано байт суммарно по всем этапам (work-байты — для tensor'ов
    /// stage и pack учитываются отдельно). Для отображения см. `display_done()`.
    pub bytes_done: u64,
    /// Всего байт «работы» в плане (2 × tensors + files). 0 пока Plan не пришёл.
    /// Используется для расчёта `fraction()`.
    pub bytes_total: u64,
    /// Реальный размер payload'а (tensors + files, без удвоения). Для
    /// человекочитаемого «X / Y ГБ» в UI.
    pub payload_total: u64,
    /// True после `ProgressEvent::Finalizing` — показываем «Финализация…»
    /// без процента (cdir+footer быстро).
    pub finalizing: bool,
    /// Человекочитаемое описание этапа для подписи (например
    /// «Сжатие 1 из 3 · tensors:lm»).
    pub stage_label: String,
}

impl CreateProgress {
    pub fn fraction(&self) -> f32 {
        if self.bytes_total == 0 {
            return 0.0;
        }
        (self.bytes_done as f64 / self.bytes_total as f64)
            .clamp(0.0, 1.0) as f32
    }
    /// «Сколько обработано» в шкале payload-байтов (для UI «X / Y ГБ»):
    /// проекция текущей fraction на полный размер модели.
    pub fn display_done(&self) -> u64 {
        if self.payload_total == 0 {
            return 0;
        }
        (self.fraction() as f64 * self.payload_total as f64) as u64
    }
}

/// Запись о `.syn` файле в выбранной папке (без открытия — лёгкое сканирование).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SynFileEntry {
    pub path: PathBuf,
    /// Имя без расширения (для отображения карточкой).
    pub display_name: String,
    /// Размер на диске в байтах. None — `std::fs::metadata` упал (битый симлинк и т. п.).
    pub size: Option<u64>,
}

/// Сводная статистика открытого пакета — считается один раз при `open_bundle`
/// и обновляется при reload. Все числа доступны без повторного парсинга cdir.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BundleStats {
    pub total_size: u64,
    pub alive_chunks: usize,
    pub tombstoned_chunks: usize,
    pub tensor_chunks: usize,
    pub quantized_chunks: usize,
    pub file_chunks: usize,
    pub ref_chunks: usize,
    /// Версия формата `(major, minor)`.
    pub format_version: (u16, u16),
}

/// Один файл-чанк внутри пакета для отображения в TableView таба «Файлы».
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntryView {
    pub name: String,
    pub kind: ChunkType,
    pub size: u64,
    pub crc32c: u32,
    /// Sha256 первые 8 байт hex (если sha256-manifest есть в bundle).
    pub sha256_short: Option<String>,
    pub tag: Option<FileTag>,
    pub alive: bool,
}

/// Запланированная (но не закоммиченная) правка содержимого пакета.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingOp {
    AddFile {
        name: String,
        data: Arc<Vec<u8>>,
        tag: FileTag,
    },
    RemoveFile {
        name: String,
    },
    Rename {
        old: String,
        new: String,
    },
}

impl PendingOp {
    pub fn description(&self) -> String {
        match self {
            PendingOp::AddFile { name, .. } => format!("+ {name}"),
            PendingOp::RemoveFile { name } => format!("− {name}"),
            PendingOp::Rename { old, new } => format!("{old} → {new}"),
        }
    }
}

/// Тип модального диалога. Хранится в `SynExplorerCtx.pending_dialog`.
/// Содержит только plain-данные (без `RwSignal`), чтобы реализовать
/// `PartialEq` — требуется `RwSignal::set`. Сами TextField'ы пишут в
/// отдельные сигналы в `SynExplorerCtx` (см. `rename_buffer`,
/// `new_package_form`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogKind {
    NewPackage,
    ConfirmDeleteFile { name: String },
    ConfirmCloseUnsaved,
    RenameFile { old: String },
    Error { title: String, message: String },
}

/// Один компонент multi-tensor пакета. Каждый компонент → отдельный
/// `tensors:<name>`-чанк в `.syn`. Один компонент = одиночная безымянная
/// модель (имя `"main"`), несколько — multi-tensor (OmniVoice lm+codec).
#[derive(Clone, Copy)]
pub struct NewPackageComponent {
    /// Суффикс tensors-чанка (без `tensors:` префикса). Пустое имя — UI
    /// валидирует и не даёт сохранить.
    pub name: RwSignal<String>,
    /// Папка с safetensors (model.safetensors / index.json / glob).
    pub source_dir: RwSignal<Option<PathBuf>>,
    /// Необязательный prefix для имён тензоров. Пустое = None.
    pub prefix: RwSignal<String>,
}

impl NewPackageComponent {
    pub fn new(name: &str) -> Self {
        Self {
            name: use_signal(name.to_string()),
            source_dir: use_signal(None),
            prefix: use_signal(String::new()),
        }
    }
}

/// Состояние формы создания нового пакета. Каждое поле — отдельный
/// `RwSignal`, чтобы `TextField::on_change` мог писать без borrow.
#[derive(Clone, Copy)]
pub struct NewPackageForm {
    pub id: RwSignal<String>,
    pub version: RwSignal<String>,
    pub arch: RwSignal<String>,
    pub purpose: RwSignal<String>,
    /// Список компонент: один = single-tensor бандл, несколько = multi-tensor.
    /// `Vec<NewPackageComponent>` (не Copy) — поэтому в `RwSignal<Vec<_>>`.
    pub components: RwSignal<Vec<NewPackageComponent>>,
    /// Целевой путь `.syn`. По умолчанию формируется как
    /// `<source_dir>.syn` после выбора source первого компонента.
    pub out_path: RwSignal<Option<PathBuf>>,
    /// Чекбокс «удалять исходные файлы после успешной паковки». По умолчанию
    /// `false` — деструктивная опция требует явного подтверждения пользователя.
    pub delete_sources: RwSignal<bool>,
}

impl NewPackageForm {
    pub fn new() -> Self {
        Self {
            id: use_signal(String::new()),
            version: use_signal("1.0.0".to_string()),
            arch: use_signal(String::new()),
            purpose: use_signal(String::new()),
            components: use_signal(vec![NewPackageComponent::new("main")]),
            out_path: use_signal(None),
            delete_sources: use_signal(false),
        }
    }
}

impl Default for NewPackageForm {
    fn default() -> Self {
        Self::new()
    }
}

/// Один открытый пакет. `Copy` через RwSignal-поля (как `CodeSession`).
/// `Bundle` сам в реестре `bundle_io::registry` — этот struct хранит только
/// реактивную репрезентацию для UI. `PartialEq` не реализуем — `RwSignal` без
/// PartialEq в syngui; для `RwSignal<Option<OpenBundle>>::set` используется
/// `set_always`.
#[derive(Clone, Copy)]
pub struct OpenBundle {
    /// Путь к `.syn` на диске (стабильный ключ для registry).
    pub path: RwSignal<PathBuf>,
    /// Редактируемая копия BundleMeta. on_change в Metadata-табе пишет сюда;
    /// сравнение с `original_meta` даёт dirty-флаг для UI.
    pub meta: RwSignal<BundleMeta>,
    /// Снапшот meta на момент последнего успешного open/save.
    pub original_meta: RwSignal<BundleMeta>,
    /// Список файлов внутри пакета для таба «Файлы».
    pub files: RwSignal<Vec<FileEntryView>>,
    pub stats: RwSignal<BundleStats>,
    pub dir_tree: RwSignal<Vec<TreeNode>>,
    /// Выбранный путь в TreeView (полное имя файла внутри пакета). None —
    /// пользователь ещё не кликал на узел.
    pub selected_path: RwSignal<Option<String>>,
    /// Кеш прочитанных payload'ов для preview — ключ = имя файла, значение =
    /// `Arc<Vec<u8>>` (read_file через registry).
    pub preview_cache: RwSignal<HashMap<String, Arc<Vec<u8>>>>,
    /// Лог изменений до commit. Пустой = нет изменений в содержимом.
    pub pending_ops: RwSignal<Vec<PendingOp>>,
    /// Композит: `meta != original_meta || !pending_ops.is_empty()`.
    pub dirty: RwSignal<bool>,
    /// Bump-счётчик: при reload инкрементируется → Reactive в табах
    /// перестраивает UI с актуальными данными.
    pub reload_gen: RwSignal<u64>,
}

impl OpenBundle {
    /// Свежий handle с пустыми сигналами. Заполняется в `bundle_io::open_async`
    /// по факту успешного чтения cdir.
    pub fn new(
        path: PathBuf,
        meta: BundleMeta,
        files: Vec<FileEntryView>,
        stats: BundleStats,
        dir_tree: Vec<TreeNode>,
    ) -> Self {
        let original = meta.clone();
        Self {
            path: use_signal(path),
            meta: use_signal(meta),
            original_meta: use_signal(original),
            files: use_signal(files),
            stats: use_signal(stats),
            dir_tree: use_signal(dir_tree),
            selected_path: use_signal(None),
            preview_cache: use_signal(HashMap::new()),
            pending_ops: use_signal(Vec::new()),
            dirty: use_signal(false),
            reload_gen: use_signal(0u64),
        }
    }
}

/// Глобальный контекст страницы SynExplorer. Создаётся один раз в `run_desktop`,
/// проносится через `provide_context`. `Copy` через RwSignal-поля.
#[derive(Clone, Copy)]
pub struct SynExplorerCtx {
    /// Закладки-папки (persisted в `AppConfig.syn_explorer_bookmarks`).
    pub bookmarks: RwSignal<Vec<PathBuf>>,
    /// Сейчас выбранная закладка в левой панели (подсветка).
    pub selected_folder: RwSignal<Option<PathBuf>>,
    /// `.syn` файлы из `selected_folder`, обновляется при выборе закладки и
    /// при ручном refresh.
    pub folder_entries: RwSignal<Vec<SynFileEntry>>,
    /// Открытый пакет (single-active в MVP).
    pub active_bundle: RwSignal<Option<OpenBundle>>,
    pub current_tab: RwSignal<TabKind>,
    pub pending_dialog: RwSignal<Option<DialogKind>>,
    pub load_state: RwSignal<LoadState>,
    pub left_split_ratio: RwSignal<f32>,
    pub right_split_ratio: RwSignal<f32>,
    /// Буфер TextField для диалога переименования файла внутри пакета.
    /// Открытие RenameFile сбрасывает сюда новое имя; commit — пушит
    /// `PendingOp::Rename`.
    pub rename_buffer: RwSignal<String>,
    /// Постоянная форма для NewPackage. Поля сбрасываются на открытии диалога
    /// через `reset_new_package_form`. Долгоживущая (как `pending_dialog`)
    /// — `DialogKind::NewPackage` не хранит сигналы внутри, чтобы оставаться
    /// `PartialEq`.
    pub new_package_form: NewPackageForm,
    /// Прогресс сжатия `.syn`. `Arc<Mutex>` обёрнут в RwSignal только
    /// чтобы `SynExplorerCtx` остался `Copy` (`Arc<Mutex>` не Copy). Сам
    /// payload-write идёт без request_redraw: worker мутирует Mutex напрямую,
    /// UI читает по триггеру `create_progress_gen` (см. ниже).
    pub create_progress: RwSignal<Arc<Mutex<CreateProgress>>>,
    /// Throttle-счётчик: worker инкрементирует ≈ раз в 60 мс (или по
    /// событиям ItemStart/Done), UI подписывается через `Reactive`. Это
    /// разводит fast-path (write_all) от reactive-системы.
    pub create_progress_gen: RwSignal<u64>,
}

impl SynExplorerCtx {
    /// Создаёт менеджер из persisted-конфига.
    pub fn new(cfg: &AppConfig) -> Self {
        let bookmarks: Vec<PathBuf> = cfg
            .syn_explorer_bookmarks
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.exists() && p.is_dir())
            .collect();
        Self {
            bookmarks: use_signal(bookmarks),
            selected_folder: use_signal(None),
            folder_entries: use_signal(Vec::new()),
            active_bundle: use_signal(None),
            current_tab: use_signal(TabKind::Overview),
            pending_dialog: use_signal(None),
            load_state: use_signal(LoadState::Idle),
            left_split_ratio: use_signal(cfg.syn_explorer_left_split_ratio),
            right_split_ratio: use_signal(cfg.syn_explorer_right_split_ratio),
            rename_buffer: use_signal(String::new()),
            new_package_form: NewPackageForm::new(),
            create_progress: use_signal(Arc::new(Mutex::new(CreateProgress::default()))),
            create_progress_gen: use_signal(0u64),
        }
    }

    /// Сбросить форму нового пакета (на открытие диалога).
    pub fn reset_new_package_form(&self) {
        let f = &self.new_package_form;
        f.id.set(String::new());
        f.version.set("1.0.0".to_string());
        f.arch.set(String::new());
        f.purpose.set(String::new());
        // `set_always` — `NewPackageComponent` не реализует `PartialEq`
        // (RwSignal-поля не сравниваются), поэтому обычный `set` не подходит.
        f.components
            .set_always(vec![NewPackageComponent::new("main")]);
        f.out_path.update(|v| *v = None);
        f.delete_sources.set(false);
        // Сбрасываем прогресс — мог остаться слепок предыдущей операции.
        let progress = self.create_progress.get_untracked();
        if let Ok(mut g) = progress.lock() {
            *g = CreateProgress::default();
        }
        self.create_progress_gen.set(0);
    }

    /// Показать модальный диалог. Перезаписывает существующий (диалоги
    /// монопольные).
    pub fn open_dialog(&self, kind: DialogKind) {
        self.pending_dialog.set(Some(kind));
    }

    /// Закрыть модальный диалог. Идемпотентно.
    pub fn close_dialog(&self) {
        if self.pending_dialog.get_untracked().is_some() {
            self.pending_dialog.set(None);
        }
    }

    /// Поднять Error-диалог с заголовком/сообщением. Альтернатива panic'у.
    pub fn show_error(&self, title: impl Into<String>, message: impl Into<String>) {
        self.pending_dialog.set(Some(DialogKind::Error {
            title: title.into(),
            message: message.into(),
        }));
    }

    /// Активный пакет (с подпиской). None = пользователь ещё не открыл.
    pub fn active(&self) -> Option<OpenBundle> {
        self.active_bundle.get()
    }

    /// Активный пакет без подписки — для callback'ов.
    pub fn active_untracked(&self) -> Option<OpenBundle> {
        self.active_bundle.get_untracked()
    }
}

/// Установить effect, синхронизирующий `dirty` при изменении meta/pending_ops.
/// Вызывается из `bundle_io::open_async` после создания `OpenBundle` —
/// до этого момента подписки не на что вешать.
pub fn install_dirty_tracker(bundle: OpenBundle) {
    create_effect(move || {
        let meta = bundle.meta.get();
        let original = bundle.original_meta.get();
        let ops_empty = bundle.pending_ops.get().is_empty();
        let meta_changed = !meta_equal(&meta, &original);
        let dirty = meta_changed || !ops_empty;
        if bundle.dirty.get_untracked() != dirty {
            bundle.dirty.set(dirty);
        }
    });
}

/// Сравнение `BundleMeta` через JSON-сериализацию: `BundleMeta` не реализует
/// `PartialEq` (см. `synaptix/crates/synaptix-bundle/src/cdir.rs`), но детерминированный
/// JSON даёт корректное сравнение по содержимому (BTreeMap/Vec сериализуются
/// в стабильном порядке, `ciborium::Value` в `extra` сериализуется через
/// `Serialize`-impl). Cтоимость сравнения — пара мс на типичный meta;
/// вызывается только в effect'е dirty-tracker.
fn meta_equal(a: &BundleMeta, b: &BundleMeta) -> bool {
    match (serde_json::to_vec(a), serde_json::to_vec(b)) {
        (Ok(av), Ok(bv)) => av == bv,
        _ => false,
    }
}
