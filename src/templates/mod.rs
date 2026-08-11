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
    /// LTX-2.3 — генерация видео (`builtin-ltx-*`).
    Video,
    /// ACE-Step — генерация музыки (`builtin-acestep-*`).
    Music,
    /// DSP / запись / микс (`builtin-audio-*`, `-voice-*`, `-save-*`, `-mix-*`).
    Audio,
    /// Пустой граф и базовые примеры (`builtin-empty`, `builtin-simple-add`).
    Basic,
}

impl TemplateCategory {
    /// Все категории в порядке отображения в боковой панели окна.
    pub const ORDER: [TemplateCategory; 4] = [
        TemplateCategory::Video,
        TemplateCategory::Music,
        TemplateCategory::Audio,
        TemplateCategory::Basic,
    ];

    /// Категория builtin-шаблона по префиксу его `id`.
    pub fn for_template(t: &Template) -> Self {
        let id = t.id.as_str();
        if id.starts_with("builtin-ltx-") || id.starts_with("builtin-h3-") {
            TemplateCategory::Video
        } else if id.starts_with("builtin-acestep-") {
            TemplateCategory::Music
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

    pub fn label(self) -> &'static str {
        match self {
            TemplateCategory::Video => "Генерация видео",
            TemplateCategory::Music => "Музыка",
            TemplateCategory::Audio => "Аудио",
            TemplateCategory::Basic => "Базовые",
        }
    }

    /// Material-иконка раздела (см. [`crate::icons`]).
    pub fn icon(self) -> &'static str {
        match self {
            TemplateCategory::Video => crate::icons::MI_MOVIE,
            TemplateCategory::Music => crate::icons::MI_LIBRARY_MUSIC,
            TemplateCategory::Audio => crate::icons::MI_GRAPHIC_EQ,
            TemplateCategory::Basic => crate::icons::MI_APPS,
        }
    }
}
