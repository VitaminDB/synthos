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
use synaptix_bundle::inspect::{LayerGroup, LayerRole, QuantKind, SizeEstimate};
use synaptix_bundle::pack_plan::PackPlan;
use synaptix_bundle::{BundleMeta, ChunkType, FileTag};

use crate::config::AppConfig;

/// Активный таб в центре страницы.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    Overview,
    Layers,
    Files,
    Metadata,
    Preview,
}

impl TabKind {
    pub fn label(self) -> String {
        match self {
            TabKind::Overview => tr!("explorer.tab.overview"),
            TabKind::Layers => tr!("explorer.tab.layers"),
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

/// Модель, которую ещё можно упаковать: каталог HuggingFace или одиночный
/// `.safetensors`. Заполняется `pack_plan::scan_collection` — без чтения
/// весов, только метаданные файлов и `config.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceEntry {
    pub path: PathBuf,
    pub display_name: String,
    pub bytes: u64,
    pub shard_count: usize,
    pub component_count: usize,
    /// `model_type` из конфига; пусто — определится при построении плана.
    pub arch: String,
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
    /// Быстрая карточка: «вот что распознано, вот куда положу — Собрать».
    /// Путь — источник, план лежит в [`PackWizard::plan`].
    PackConfirm,
    /// Полный мастер из трёх шагов (кнопка «Настроить…»).
    PackWizard,
    ConfirmDeleteFile { name: String },
    ConfirmCloseUnsaved,
    RenameFile { old: String },
    Error { title: String, message: String },
}

/// Слои одного `tensors:*`-чанка — то, что показывает вкладка «Слои» и
/// шаг «Состав» мастера. Считается при открытии пакета: разбор
/// safetensors-заголовка внутри mmap стоит миллисекунды даже на 77 ГБ.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentLayers {
    /// Имя компонента без префикса `tensors:`.
    pub component: String,
    pub tensor_count: usize,
    pub bytes: u64,
    /// Вес по ролям, от большего к меньшему — для полосы состава.
    /// Каждая запись знает и размер после кванта.
    pub by_role: Vec<(LayerRole, SizeEstimate)>,
    pub groups: Vec<LayerGroup>,
}

/// Выбранная точность для роли слоёв. `Dense` — не квантовать.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuantChoice {
    Dense,
    Nvfp4,
    Mxfp8,
    /// SQ`b` — одноблобный формат движка на 1..=8 бит, любая карта sm_80+.
    Sq(u8),
}

impl QuantChoice {
    /// Все варианты в порядке показа: плотно, нативные Blackwell-форматы,
    /// затем SQ от точного к компактному.
    pub const ALL: &'static [QuantChoice] = &[
        QuantChoice::Dense,
        QuantChoice::Nvfp4,
        QuantChoice::Mxfp8,
        QuantChoice::Sq(8),
        QuantChoice::Sq(6),
        QuantChoice::Sq(5),
        QuantChoice::Sq(4),
        QuantChoice::Sq(3),
        QuantChoice::Sq(2),
    ];

    /// Значение для `BundleMeta.extra` и для выбора ядра при упаковке.
    pub fn key(self) -> String {
        match self {
            QuantChoice::Dense => "dense".into(),
            QuantChoice::Nvfp4 => "nvfp4".into(),
            QuantChoice::Mxfp8 => "mxfp8".into(),
            QuantChoice::Sq(b) => format!("sq{b}"),
        }
    }
    /// Формат для оценки размера; `None` — плотные веса.
    pub fn kind(self) -> Option<QuantKind> {
        match self {
            QuantChoice::Dense => None,
            QuantChoice::Nvfp4 => Some(QuantKind::Nvfp4),
            QuantChoice::Mxfp8 => Some(QuantKind::Mxfp8),
            QuantChoice::Sq(b) => Some(QuantKind::Sq(b)),
        }
    }
    pub fn from_key(s: &str) -> Self {
        match s {
            "nvfp4" => QuantChoice::Nvfp4,
            "mxfp8" => QuantChoice::Mxfp8,
            other => match other.strip_prefix("sq").and_then(|b| b.parse::<u8>().ok()) {
                Some(b) if (1..=8).contains(&b) => QuantChoice::Sq(b),
                _ => QuantChoice::Dense,
            },
        }
    }
    pub fn label(self) -> String {
        match self {
            QuantChoice::Dense => tr!("explorer.quant.dense"),
            QuantChoice::Nvfp4 => "NVFP4".to_string(),
            QuantChoice::Mxfp8 => "MXFP8".to_string(),
            QuantChoice::Sq(b) => format!("SQ{b}"),
        }
    }
}

/// Точности, которые движок реально умеет применить к этой роли.
///
/// Список задан не форматом `.syn`, а тем, что делает загрузчик
/// (`synaptix-llm-common::model::build_ext` и `PrecisionConfig`):
/// * внимание, MLP и `lm_head` идут через `QLinear::build` — NVFP4, MXFP8 и
///   SQ1…SQ8 (энкодер SQ на карте, этап 4 плана низкобитных квантов);
/// * эмбеддинги — MXFP8 и SQ: у них есть gather; NVFP4-ядра gather'а нет,
///   а пресет `PrecisionConfig::nvfp4()` и вовсе оставляет эмбеддинги в F16;
/// * остальное (свёртки, нормировки, vision-башня, модуляция DiT) движок
///   отдельной ручкой не адресует — предлагать там выбор значило бы врать.
pub fn allowed_quants(role: LayerRole) -> &'static [QuantChoice] {
    match role {
        LayerRole::Attention | LayerRole::Mlp | LayerRole::LmHead => QuantChoice::ALL,
        // Эмбеддинги: gather есть у MXFP8 и SQ (одно ядро с блоками GGUF),
        // у NVFP4 — нет.
        LayerRole::Embedding => &[
            QuantChoice::Dense,
            QuantChoice::Mxfp8,
            QuantChoice::Sq(8),
            QuantChoice::Sq(6),
            QuantChoice::Sq(4),
        ],
        _ => &[],
    }
}

pub fn role_is_quantizable(role: LayerRole) -> bool {
    !allowed_quants(role).is_empty()
}

/// Состояние упаковки: разобранный план плюс правки пользователя поверх.
///
/// Сам [`PackPlan`] — plain-данные из `synaptix-bundle`; он не `Copy`, поэтому
/// живёт в `Arc`. Правки не мутируют план, а лежат рядом: так «Сбросить»
/// сводится к очистке нескольких сигналов, а не к повторному скану диска.
#[derive(Clone, Copy)]
pub struct PackWizard {
    /// Что паковать. None — источник ещё не выбран или идёт скан.
    pub plan: RwSignal<Option<Arc<PackPlan>>>,
    /// Идёт разбор источника — карточка показывает индикатор вместо состава.
    pub scanning: RwSignal<bool>,
    /// Текущий шаг мастера (0..=2).
    pub step: RwSignal<usize>,
    pub id: RwSignal<String>,
    pub version: RwSignal<String>,
    pub arch: RwSignal<String>,
    pub purpose: RwSignal<String>,
    /// Куда писать. Заполняется каталогом моделей из настроек.
    pub out_path: RwSignal<Option<PathBuf>>,
    /// Деструктивная опция — по умолчанию выключена.
    pub delete_sources: RwSignal<bool>,
    /// Флаги «паковать ли» по индексам `plan.components` / `plan.aux`.
    pub components_enabled: RwSignal<Vec<bool>>,
    pub aux_enabled: RwSignal<Vec<bool>>,
    /// Слои главного компонента — инспектор внутри мастера.
    pub layers: RwSignal<Option<ComponentLayers>>,
    /// Выбранная точность по ролям слоёв.
    pub quant: RwSignal<Vec<(LayerRole, QuantChoice)>>,
    /// Точность для отдельных групп слоёв (ключ — `LayerGroup::pattern`).
    /// Перекрывает выбор по роли; заполняется только в режиме эксперта.
    pub quant_groups: RwSignal<Vec<(String, QuantChoice)>>,
    /// Режим эксперта: префиксы тензоров, пофайловый состав, точность по
    /// группам, контрольные суммы. Липкий — хранится в конфиге.
    pub expert: RwSignal<bool>,
    /// Считать SHA-256 по каждому чанку и манифест. Дорого на больших
    /// моделях, поэтому по умолчанию выключено.
    pub sha256: RwSignal<bool>,
    /// Blake3 — то же, но в 5–10 раз быстрее.
    pub blake3: RwSignal<bool>,
    /// Писать центральный каталог как JSON вместо CBOR: читаемо глазами,
    /// удобно при разборе полётов, чуть больше по размеру.
    pub cdir_json: RwSignal<bool>,
}

impl PackWizard {
    pub fn new() -> Self {
        Self {
            plan: use_signal(None),
            scanning: use_signal(false),
            step: use_signal(0usize),
            id: use_signal(String::new()),
            version: use_signal(String::new()),
            arch: use_signal(String::new()),
            purpose: use_signal(String::new()),
            out_path: use_signal(None),
            delete_sources: use_signal(false),
            components_enabled: use_signal(Vec::new()),
            aux_enabled: use_signal(Vec::new()),
            layers: use_signal(None),
            quant: use_signal(Vec::new()),
            quant_groups: use_signal(Vec::new()),
            expert: use_signal(false),
            sha256: use_signal(false),
            blake3: use_signal(false),
            cdir_json: use_signal(false),
        }
    }

    /// Сбросить всё к «источник не выбран».
    pub fn reset(&self) {
        self.plan.set(None);
        self.scanning.set(false);
        self.step.set(0);
        self.id.set(String::new());
        self.version.set(String::new());
        self.arch.set(String::new());
        self.purpose.set(String::new());
        self.out_path.update(|v| *v = None);
        self.delete_sources.set(false);
        self.components_enabled.update(|v| v.clear());
        self.aux_enabled.update(|v| v.clear());
        self.layers.set(None);
        self.quant.update(|v| v.clear());
        self.quant_groups.update(|v| v.clear());
        // `expert` и флаги контрольных сумм — настройки пользователя, а не
        // свойства источника: между упаковками они сохраняются.
    }

    /// Принять разобранный план: поля формы заполняются догадками, все
    /// компоненты и файлы — своими значениями из плана.
    pub fn adopt(&self, plan: Arc<PackPlan>, layers: Option<ComponentLayers>) {
        self.id.set(plan.meta.id.clone());
        self.version.set(plan.meta.version.clone());
        self.arch.set(plan.meta.arch.clone());
        self.purpose.set(plan.meta.purpose.clone());
        self.components_enabled
            .update(|v| *v = plan.components.iter().map(|c| c.enabled).collect());
        self.aux_enabled
            .update(|v| *v = plan.aux.iter().map(|f| f.enabled).collect());
        // Роли — из инспектора, порядок как в полосе состава.
        let roles: Vec<(LayerRole, QuantChoice)> = layers
            .as_ref()
            .map(|l| {
                l.by_role
                    .iter()
                    .map(|(r, _)| (*r, QuantChoice::Dense))
                    .collect()
            })
            .unwrap_or_default();
        self.quant.update(|v| *v = roles);
        self.quant_groups.update(|v| v.clear());
        self.layers.set(layers);
        self.scanning.set(false);
        self.plan.set(Some(plan));
    }

    /// Одна точность на все квантуемые роли — быстрый путь, где разбирать
    /// модель по слоям пользователь не собирается.
    pub fn set_quant_for_all(&self, choice: QuantChoice) {
        self.quant.update(|v| {
            for (role, slot) in v.iter_mut() {
                // Роль может не уметь выбранный формат (эмбеддинги и NVFP4) —
                // тогда она просто остаётся плотной, а не получает то, чего
                // движок не применит.
                if allowed_quants(*role).contains(&choice) {
                    *slot = choice;
                } else if role_is_quantizable(*role) {
                    *slot = QuantChoice::Dense;
                }
            }
        });
        self.quant_groups.update(|v| v.clear());
    }

    /// Общая точность, выбранная «одной кнопкой»: та, которую получили все
    /// роли, способные её принять, а остальные остались плотными. Если
    /// картина не сводится к такому виду — None, «смешанная».
    ///
    /// Проверка учитывает возможности ролей: эмбеддинги не умеют NVFP4, и
    /// без этой оговорки глобальный выбор «NVFP4» сразу же показывался бы
    /// как «смешанный».
    pub fn uniform_quant(&self) -> Option<QuantChoice> {
        if !self.quant_groups.get().is_empty() {
            return None;
        }
        let q = self.quant.get();
        let matches_global = |c: QuantChoice| {
            q.iter().filter(|(r, _)| role_is_quantizable(*r)).all(|(role, slot)| {
                let expected = if allowed_quants(*role).contains(&c) { c } else { QuantChoice::Dense };
                *slot == expected
            })
        };
        QuantChoice::ALL.iter().copied().find(|c| matches_global(*c))
    }

    /// Выбор для роли (с учётом того, что роль может быть неквантуемой).
    pub fn quant_for(&self, role: LayerRole) -> QuantChoice {
        if !role_is_quantizable(role) {
            return QuantChoice::Dense;
        }
        self.quant
            .get()
            .iter()
            .find(|(r, _)| *r == role)
            .map(|(_, c)| *c)
            .unwrap_or(QuantChoice::Dense)
    }

    /// Оценка итогового payload'а с учётом выбранной точности.
    ///
    /// Квант применяется к главному компоненту — тому, чьи слои разобраны;
    /// остальные компоненты и файлы входят своим размером. Это оценка, а не
    /// обещание: реальный размер зависит ещё и от выравнивания чанков.
    pub fn estimated_payload(&self) -> Option<(u64, u64)> {
        let plan = self.effective_plan()?;
        let dense_total = plan.payload_bytes();
        let Some(layers) = self.layers.get() else {
            return Some((dense_total, dense_total));
        };
        let mut quantized: u64 = 0;
        for (role, est) in layers.by_role.iter() {
            quantized = quantized.saturating_add(est.for_kind(self.quant_for(*role).kind()));
        }
        // Заменяем вес главного компонента на пересчитанный.
        let rest = dense_total.saturating_sub(layers.bytes);
        Some((dense_total, rest.saturating_add(quantized)))
    }

    /// Решение о квантовании в виде plain-данных для worker'а. Пустое —
    /// упаковка обычная, побайтовая.
    pub fn quant_decision(&self) -> crate::pages::syn_explorer::quant_pack::QuantDecision {
        use crate::pages::syn_explorer::quant_pack::QuantDecision;
        let Some(layers) = self.layers.get_untracked() else {
            return QuantDecision::default();
        };
        let plan = self.plan.get_untracked();
        let (purpose, id) = plan
            .map(|p| (p.meta.purpose.clone(), p.meta.id.clone()))
            .unwrap_or_default();
        QuantDecision {
            hint: format!("{} {purpose} {id}", layers.component),
            by_role: self
                .quant
                .get_untracked()
                .into_iter()
                .filter_map(|(r, c)| c.kind().map(|k| (r, k)))
                .collect(),
            by_group: self
                .quant_groups
                .get_untracked()
                .into_iter()
                .filter_map(|(g, c)| c.kind().map(|k| (g, k)))
                .collect(),
        }
    }

    /// План с наложенными правками — то, что реально пойдёт в упаковку.
    pub fn effective_plan(&self) -> Option<PackPlan> {
        let base = self.plan.get_untracked()?;
        let mut plan = (*base).clone();
        plan.meta.id = self.id.get_untracked().trim().to_string();
        plan.meta.version = self.version.get_untracked().trim().to_string();
        plan.meta.arch = self.arch.get_untracked().trim().to_string();
        plan.meta.purpose = self.purpose.get_untracked().trim().to_string();
        let comps = self.components_enabled.get_untracked();
        for (i, c) in plan.components.iter_mut().enumerate() {
            if let Some(on) = comps.get(i) {
                c.enabled = *on;
            }
        }
        let aux = self.aux_enabled.get_untracked();
        for (i, f) in plan.aux.iter_mut().enumerate() {
            if let Some(on) = aux.get(i) {
                f.enabled = *on;
            }
        }
        Some(plan)
    }

    /// Незаполненное обязательное поле — текст ошибки для UI, либо None.
    pub fn validation_error(&self) -> Option<String> {
        if self.plan.get_untracked().is_none() {
            return Some(tr!("explorer.error.missing_data.no_source"));
        }
        if self.id.get_untracked().trim().is_empty() {
            return Some(tr!("explorer.error.missing_data.id_required"));
        }
        if self.version.get_untracked().trim().is_empty() {
            return Some(tr!("explorer.error.missing_data.version_required"));
        }
        if self.out_path.get_untracked().is_none() {
            return Some(tr!("explorer.error.missing_data.out_path_required"));
        }
        if !self.components_enabled.get_untracked().iter().any(|v| *v) {
            return Some(tr!("explorer.error.missing_data.no_components"));
        }
        None
    }
}

impl Default for PackWizard {
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
    /// Состав каждого `tensors:*`-чанка для вкладки «Слои».
    pub layers: RwSignal<Vec<ComponentLayers>>,
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
        layers: Vec<ComponentLayers>,
        dir_tree: Vec<TreeNode>,
    ) -> Self {
        let original = meta.clone();
        Self {
            path: use_signal(path),
            meta: use_signal(meta),
            original_meta: use_signal(original),
            files: use_signal(files),
            stats: use_signal(stats),
            layers: use_signal(layers),
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
    /// Модели из той же папки, которые ещё можно упаковать.
    pub folder_sources: RwSignal<Vec<SourceEntry>>,
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
    /// Состояние упаковки. Долгоживущее (как `pending_dialog`): варианты
    /// `DialogKind` не хранят сигналов внутри, чтобы оставаться `PartialEq`.
    pub wizard: PackWizard,
    /// Каталог моделей приложения — путь по умолчанию для нового бандла.
    /// Заполняется из настроек при создании контекста.
    pub models_dir: RwSignal<PathBuf>,
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
            folder_sources: use_signal(Vec::new()),
            active_bundle: use_signal(None),
            current_tab: use_signal(TabKind::Overview),
            pending_dialog: use_signal(None),
            load_state: use_signal(LoadState::Idle),
            left_split_ratio: use_signal(cfg.syn_explorer_left_split_ratio),
            right_split_ratio: use_signal(cfg.syn_explorer_right_split_ratio),
            rename_buffer: use_signal(String::new()),
            wizard: {
                let w = PackWizard::new();
                w.expert.set(cfg.syn_explorer_expert);
                w
            },
            models_dir: use_signal(crate::config::resolve_models_dir(&cfg.models_dir)),
            create_progress: use_signal(Arc::new(Mutex::new(CreateProgress::default()))),
            create_progress_gen: use_signal(0u64),
        }
    }

    /// Подготовить мастер к новой упаковке: сбросить правки и прогресс.
    pub fn reset_wizard(&self) {
        self.wizard.reset();
        // Прогресс мог остаться слепком предыдущей операции.
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
