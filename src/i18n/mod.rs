//! Каталоги строк интерфейса и связка `GeneralCtx.language` → `syngui::i18n`.
//!
//! `AppConfig.general.language` хранит `"auto"` (язык системы) или тег
//! каталога. Эффект в `install` переводит значение в `syngui::i18n::set_language`,
//! а корневой `Reactive` в `lib.rs` перестраивает оболочку при смене языка.

use syngui::i18n::Lang;
use syngui::prelude::*;
use syngui::widgets::overlay::menu::MenuItem;

use crate::agent::tools::Tool;
use crate::context::GeneralCtx;
use crate::icons::MI_CHECK;
use crate::pages::node_editor::registry::{NodeCategory, NodeKindMeta};
use crate::pages::node_editor::types::NodeKind;
use crate::templates::{Template, TemplateCategory};

pub const AUTO: &str = "auto";

pub const CATALOGS: [&str; 14] = [
    include_str!("../../i18n/en.lang"),
    include_str!("../../i18n/ru.lang"),
    include_str!("../../i18n/de.lang"),
    include_str!("../../i18n/fr.lang"),
    include_str!("../../i18n/es.lang"),
    include_str!("../../i18n/it.lang"),
    include_str!("../../i18n/pt-BR.lang"),
    include_str!("../../i18n/pl.lang"),
    include_str!("../../i18n/uk.lang"),
    include_str!("../../i18n/kk.lang"),
    include_str!("../../i18n/tr.lang"),
    include_str!("../../i18n/zh-CN.lang"),
    include_str!("../../i18n/ja.lang"),
    include_str!("../../i18n/ko.lang"),
];

pub fn install(general: GeneralCtx) {
    syngui::i18n::register_catalogs(&CATALOGS);
    create_effect(move || {
        let raw = general.language.get();
        syngui::i18n::set_language(resolve(&raw));
    });
}

fn resolve(raw: &str) -> Lang {
    if raw == AUTO {
        syngui::i18n::system_language()
    } else {
        Lang::new(raw)
    }
}

/// Родное название языка системы, если для него есть каталог; иначе его тег.
pub fn system_language_name() -> String {
    let sys = syngui::i18n::system_language();
    syngui::i18n::languages()
        .into_iter()
        .find(|l| l.tag == sys || l.tag.base() == sys.base())
        .map(|l| l.name)
        .unwrap_or_else(|| sys.tag().to_string())
}

pub fn language_items() -> Vec<DropdownItem> {
    let mut items = vec![DropdownItem::new(
        AUTO,
        tr!("settings.general.language.auto", name = system_language_name()),
    )];
    items.extend(
        syngui::i18n::languages()
            .into_iter()
            .map(|l| DropdownItem::new(l.tag.tag(), l.name)),
    );
    items
}

pub fn language_menu_items(current: &str) -> Vec<MenuItem> {
    let mark = |item: MenuItem, id: &str| if id == current { item.icon(MI_CHECK) } else { item };
    let mut items = vec![mark(
        MenuItem::new(AUTO, tr!("nav.language.auto", name = system_language_name())),
        AUTO,
    )];
    for l in syngui::i18n::languages() {
        let id = l.tag.tag().to_string();
        items.push(mark(MenuItem::new(id.clone(), l.name), &id));
    }
    items
}

/// serde-имя варианта `NodeKind` (snake_case) — стабильный ключ каталога.
pub fn node_kind_key(kind: NodeKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Заголовок ноды: `node.<kind>.title`, иначе английский литерал из реестра.
pub fn node_title(meta: &NodeKindMeta) -> String {
    syngui::i18n::try_tr(&format!("node.{}.title", node_kind_key(meta.kind)))
        .unwrap_or_else(|| meta.title.to_string())
}

pub fn node_category_label(category: NodeCategory) -> String {
    tr!(&format!("node.category.{}", category.key()))
}

pub fn node_subcategory_label(key: &str) -> String {
    tr!(&format!("node.subcategory.{key}"))
}

pub fn template_category_label(category: TemplateCategory) -> String {
    tr!(&format!("template.category.{}", category.key()))
}

/// Имя встроенного шаблона: `template.<id>.name`, иначе как в `builtin.rs`;
/// пользовательские шаблоны показываются как названы.
pub fn template_name(template: &Template) -> String {
    if template.builtin {
        syngui::i18n::try_tr(&format!("template.{}.name", template.id))
            .unwrap_or_else(|| template.name.clone())
    } else {
        template.name.clone()
    }
}

pub fn template_description(template: &Template) -> String {
    if template.builtin {
        syngui::i18n::try_tr(&format!("template.{}.desc", template.id))
            .unwrap_or_else(|| template.description.clone())
    } else {
        template.description.clone()
    }
}

/// Подпись инструмента агента: `tool.<key>.label`, иначе литерал из дескриптора.
pub fn tool_label(tool: &Tool) -> String {
    syngui::i18n::try_tr(&format!("tool.{}.label", tool.key)).unwrap_or_else(|| tool.label.to_string())
}
