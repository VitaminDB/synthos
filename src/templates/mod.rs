//! Шаблоны графа нод. Builtin (in-code) + custom (`~/.config/synthos/templates/*.json`).
//!
//! Builtin не лежат на диске: они генерируются в [`builtin::all`] на старте
//! приложения и недоступны для удаления/переименования. Custom — обычные
//! JSON-файлы, slug = id = имя файла без расширения.
//!
//! Загрузка / снэпшот графа реализованы в [`convert`] (NodeInstance ↔
//! сериализуемый NodeData) — UI не трогает FieldValue/RwSignal'ы напрямую,
//! а ходит через эти helper'ы.

pub mod builtin;
pub mod convert;
pub mod model;
pub mod storage;

pub use model::{ConnData, FieldValueData, NodeData, PointData, Template, TemplateKind, ViewportData};
pub use storage::{
    create, delete, duplicate_to_custom, load_all as load_custom, load_one, rename, save,
    templates_dir, TemplateError,
};

/// Полный список шаблонов: builtin сверху, custom снизу. Builtin —
/// статичный, custom загружается с диска.
pub fn list_all() -> Vec<Template> {
    let mut out = builtin::all();
    out.extend(load_custom());
    out
}

/// Раздел встроенных шаблонов в окне выбора. Выводится из стабильного
/// `id`-префикса builtin-шаблонов (модель `Template` без поля категории).
/// Пользовательские (`builtin=false`) шаблоны показываются в отдельной
/// секции «Свои» — она не входит в этот enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateCategory {
    /// LTX-2.3 / MiniMax-H3 — генерация видео (`builtin-ltx-*`, `builtin-h3-*`).
    Video,
    /// FLUX.1 / FLUX.2 / Qwen-Image / SDXL — картинки (`builtin-flux-*`,
    /// `builtin-flux2-*`, `builtin-qwen-image*`, `builtin-sdxl-*`).
    Image,
    /// ACE-Step / YuE2 — генерация музыки (`builtin-acestep-*`, `builtin-yue2-*`).
    Music,
    /// VoxCPM / OmniVoice / VibeVoice — синтез и клонирование речи
    /// (`builtin-voice-clone-*`, `builtin-vibevoice-*`).
    Speech,
    /// DSP / запись / микс (`builtin-audio-*`, `builtin-voice-recording`,
    /// `-save-*`, `-mix-*`).
    Audio,
    /// Пустой граф и базовые примеры (`builtin-empty`, `builtin-simple-add`).
    Basic,
}

impl TemplateCategory {
    /// Все категории в порядке отображения в боковой панели окна.
    pub const ORDER: [TemplateCategory; 6] = [
        TemplateCategory::Video,
        TemplateCategory::Image,
        TemplateCategory::Music,
        TemplateCategory::Speech,
        TemplateCategory::Audio,
        TemplateCategory::Basic,
    ];

    /// Категория builtin-шаблона по префиксу его `id`.
    pub fn for_template(t: &Template) -> Self {
        let id = t.id.as_str();
        if id.starts_with("builtin-ltx-") || id.starts_with("builtin-h3-") {
            TemplateCategory::Video
        } else if id.starts_with("builtin-flux-")
            || id.starts_with("builtin-flux2-")
            || id.starts_with("builtin-qwen-image-")
            || id.starts_with("builtin-qwen-image21-")
            || id.starts_with("builtin-sdxl-")
        {
            TemplateCategory::Image
        } else if id.starts_with("builtin-acestep-") || id.starts_with("builtin-yue2-") {
            TemplateCategory::Music
        } else if id.starts_with("builtin-voice-clone-") || id.starts_with("builtin-vibevoice-") {
            // До «Аудио»: `builtin-voice-` там — запись с микрофона.
            TemplateCategory::Speech
        } else if id.starts_with("builtin-audio-")
            || id.starts_with("builtin-voice-")
            || id.starts_with("builtin-save-")
            || id.starts_with("builtin-mix-")
        {
            TemplateCategory::Audio
        } else {
            TemplateCategory::Basic
        }
    }

    /// Стабильный ключ для каталога строк.
    pub fn key(self) -> &'static str {
        match self {
            TemplateCategory::Video => "video",
            TemplateCategory::Image => "image",
            TemplateCategory::Music => "music",
            TemplateCategory::Speech => "speech",
            TemplateCategory::Audio => "audio",
            TemplateCategory::Basic => "basic",
        }
    }

    /// Material-иконка раздела (см. [`crate::icons`]).
    pub fn icon(self) -> &'static str {
        match self {
            TemplateCategory::Video => crate::icons::MI_MOVIE,
            TemplateCategory::Image => crate::icons::MI_IMAGE_ICON,
            TemplateCategory::Music => crate::icons::MI_LIBRARY_MUSIC,
            TemplateCategory::Speech => crate::icons::MI_RECORD_VOICE_OVER,
            TemplateCategory::Audio => crate::icons::MI_GRAPHIC_EQ,
            TemplateCategory::Basic => crate::icons::MI_APPS,
        }
    }
}
