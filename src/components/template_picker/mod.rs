//! Всплывающее окно выбора шаблонов графа нод.
//!
//! Модальный [`Portal`] по центру экрана. Открывается кнопкой «+» в баре
//! вкладок node-editor'а (сигнал [`EditorWorkspace::template_picker_open`]),
//! закрывается по backdrop / Escape / крестику / выбору шаблона.
//!
//! Внутри:
//! - **шапка** — заголовок + «Сохранить текущий граф» / «Сохранить как
//!   новый» / «×»;
//! - **боковая панель** — 5 разделов: Генерация видео / Музыка / Аудио /
//!   Базовые ([`TemplateCategory`]) + «Свои» (custom);
//! - **сетка карточек** справа (см. [`template_card`]), фильтруется по
//!   выбранному разделу.
//!
//! Смонтирован один раз в top-level `Stack` shell'а — Portal сам уходит в
//! overlay-слой движка, поэтому виден только когда сигнал открытости true.

pub mod editable_label;
pub mod template_card;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::dialog::{Dialog, DialogAction};
use syngui::widgets::overlay::portal::{Portal, PortalAnchor};
use syngui::widgets::{
    DecoratedBox, Grid, GestureDetector, Padding, Reactive, Row, ScrollView, ToolButton,
};

use crate::context::AppCtx;
use crate::icons::{MI_ADD, MI_CLOSE, MI_FOLDER, MI_SAVE};
use crate::pages::node_editor::tabs::EditorWorkspace;
use crate::templates::{self, Template, TemplateCategory};

/// Индекс раздела «Свои» = после всех builtin-категорий.
const CUSTOM_SECTION: usize = TemplateCategory::ORDER.len();

/// Точка входа: модальный Porter выбора шаблонов. Монтируется в shell.
pub fn view() -> impl Widget {
    let ws = use_context::<EditorWorkspace>();
    let picker_open = ws.template_picker_open;
    let selected_section = use_signal(0_usize);

    Portal::new()
        .is_open(picker_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .width(920.0)
        .on_close(move || picker_open.set(false))
        .child(card(selected_section, picker_open))
}

fn card(selected_section: RwSignal<usize>, picker_open: RwSignal<bool>) -> impl Widget {
    let body = mgui! {
        Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                sidebar(selected_section),
                content_area(selected_section),
            ]
    };

    DecoratedBox::new()
        .child(mgui! {
            Column::new()
                .gap(0.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    DecoratedBox::new()
                        .child(Padding::only(16.0, 12.0, 12.0, 12.0).child(header_row(picker_open)))
                        .class("tpl-picker-header"),
                    DecoratedBox::new().class("tpl-picker-body").child(body),
                ]
        })
        .class("tpl-picker-card")
}

fn header_row(picker_open: RwSignal<bool>) -> impl Widget {
    mgui! {
        Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                Text::new(tr!("templates.title")).class("tpl-picker-title"),
                Row::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .main_axis_alignment(MainAxisAlignment::End)
                    .child(
                        ToolButton::new(MI_SAVE)
                            .tooltip(tr!("templates.header.save_current"))
                            .on_click(move || {
                                save_or_update_current();
                            })
                            .class("tpl-picker-action"),
                    )
                    .child(
                        ToolButton::new(MI_ADD)
                            .tooltip(tr!("templates.header.save_as_new"))
                            .on_click(move || {
                                save_current_as_template();
                            })
                            .class("tpl-picker-action"),
                    )
                    .child(
                        ToolButton::new(MI_CLOSE)
                            .tooltip(tr!("app.close"))
                            .on_click(move || {
                                picker_open.set(false);
                            })
                            .class("tpl-picker-action"),
                    ),
            ]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Боковая панель разделов
// ─────────────────────────────────────────────────────────────────────────────

fn sidebar(selected: RwSignal<usize>) -> impl Widget {
    let inner = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let cur = selected.get();
        let mut col = Column::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (i, cat) in TemplateCategory::ORDER.iter().enumerate() {
            col = col.child(section_item(
                i,
                cat.icon(),
                crate::i18n::template_category_label(*cat),
                cur == i,
                selected,
            ));
        }
        col = col.child(section_item(
            CUSTOM_SECTION,
            MI_FOLDER,
            tr!("templates.section.custom"),
            cur == CUSTOM_SECTION,
            selected,
        ));
        vec![Box::new(col)]
    });

    DecoratedBox::new()
        .child(Padding::all(8.0).child(inner))
        .class("tpl-picker-sidebar")
}

fn section_item(
    idx: usize,
    icon: &str,
    label: impl Into<String>,
    selected: bool,
    sel_sig: RwSignal<usize>,
) -> impl Widget {
    let class = if selected {
        "tpl-picker-section tpl-picker-section--selected"
    } else {
        "tpl-picker-section"
    };
    let row = mgui! {
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .main_axis_alignment(MainAxisAlignment::Start) => [
                Text::new(icon).class("tpl-picker-section-icon"),
                Text::new(label).class("tpl-picker-section-label"),
            ]
    };
    GestureDetector::new()
        .on_click(move || {
            sel_sig.set(idx);
        })
        .child(
            DecoratedBox::new()
                .child(Padding::only(10.0, 8.0, 10.0, 8.0).child(row))
                .class(class),
        )
}

// ─────────────────────────────────────────────────────────────────────────────
// Сетка карточек выбранного раздела
// ─────────────────────────────────────────────────────────────────────────────

fn content_area(selected: RwSignal<usize>) -> impl Widget {
    let list = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _rev = templates_revision().get();
        let idx = selected.get();

        let templates_to_show: Vec<Template> = if idx == CUSTOM_SECTION {
            templates::load_custom()
        } else {
            let cat = TemplateCategory::ORDER[idx.min(TemplateCategory::ORDER.len() - 1)];
            templates::builtin::all()
                .into_iter()
                .filter(|t| TemplateCategory::for_template(t) == cat)
                .collect()
        };

        if templates_to_show.is_empty() {
            return vec![Box::new(Padding::all(24.0).child(empty_state(idx == CUSTOM_SECTION)))];
        }

        let mut grid = Grid::new(3).gap(10.0);
        for t in templates_to_show {
            grid = grid.child(template_card::view(t));
        }
        vec![Box::new(Padding::all(14.0).child(grid))]
    });

    ScrollView::new().child(list).class("tpl-picker-list")
}

fn empty_state(custom: bool) -> impl Widget {
    let title = if custom {
        tr!("templates.empty.custom.title")
    } else {
        tr!("templates.empty.builtin.title")
    };
    let hint = if custom {
        tr!("templates.empty.custom.hint")
    } else {
        tr!("templates.empty.builtin.hint")
    };
    Padding::all(24.0).child(
        Column::new()
            .gap(6.0)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(title).class("tpl-picker-empty-title"))
            .child(Text::new(hint).class("tpl-picker-empty-hint")),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// templates_revision — глобальный счётчик обновлений списка custom-шаблонов
//
// При create/delete/rename caller дёргает `bump_revision()`, и все Reactive,
// читающие `templates_revision().get()`, перестраиваются.
// ─────────────────────────────────────────────────────────────────────────────

fn templates_revision() -> RwSignal<u64> {
    use std::sync::OnceLock;
    static SLOT: OnceLock<RwSignal<u64>> = OnceLock::new();
    *SLOT.get_or_init(|| use_signal(0_u64))
}

pub fn bump_revision() {
    let s = templates_revision();
    s.set(s.get_untracked() + 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Save current graph as template
// ─────────────────────────────────────────────────────────────────────────────

/// Снимает снимок активного `NodeEditorCtx` и сохраняет как новый custom-
/// шаблон с авто-именем. Пользователь сразу может переименовать через
/// двойной клик в карточке. После create — активная вкладка привязывается
/// к новому шаблону (`tab.source`), dirty/fingerprint сбрасываются.
fn save_current_as_template() {
    let ws = use_context::<EditorWorkspace>();
    let app = use_context::<AppCtx>();
    let Some(tab) = active_tab(&ws) else { return };
    let (nodes, conns, viewport) = templates::convert::snapshot(&tab.ctx);
    let mut t = Template::empty(tab.title.get_untracked(), templates::TemplateKind::Full);
    if t.name.trim().is_empty() {
        t.name = tr!("templates.default_name");
    }
    t.nodes = nodes;
    t.connections = conns;
    t.viewport = viewport;
    match templates::create(t) {
        Ok(saved) => {
            tab.source.set(Some(saved.id.clone()));
            tab.title.set(saved.name.clone());
            mark_tab_saved(&tab);
            app.notifications.success(tr!("templates.notify.saved", name = saved.name));
            bump_revision();
        }
        Err(e) => {
            app.notifications.error(tr!("templates.notify.save_failed", error = e));
        }
    }
}

/// Обновляет шаблон, из которого открыта активная вкладка (`tab.source`).
/// Если вкладка не привязана (Untitled) ИЛИ привязана к builtin — fallback
/// на «Save as new».
fn save_or_update_current() {
    let ws = use_context::<EditorWorkspace>();
    let app = use_context::<AppCtx>();
    let Some(tab) = active_tab(&ws) else { return };

    let Some(src_id) = tab.source.get_untracked() else {
        save_current_as_template();
        return;
    };
    let Some(existing) = templates::storage::load_one(&src_id) else {
        save_current_as_template();
        return;
    };
    if existing.builtin {
        save_current_as_template();
        return;
    }

    let (nodes, conns, viewport) = templates::convert::snapshot(&tab.ctx);
    let updated = Template {
        id: existing.id.clone(),
        builtin: false,
        name: tab.title.get_untracked(),
        description: existing.description.clone(),
        kind: existing.kind,
        nodes,
        connections: conns,
        viewport,
    };
    match templates::storage::save(&updated) {
        Ok(()) => {
            mark_tab_saved(&tab);
            app.notifications.success(tr!("templates.notify.updated", name = updated.name));
            bump_revision();
        }
        Err(e) => {
            app.notifications.error(tr!("templates.notify.update_failed", error = e));
        }
    }
}

fn active_tab(ws: &EditorWorkspace) -> Option<crate::pages::node_editor::tabs::OpenTab> {
    let id = ws.active.get_untracked()?;
    ws.tabs.get_untracked().into_iter().find(|t| t.id == id)
}

/// После успешного save: пересчитать fingerprint и сбросить dirty.
fn mark_tab_saved(tab: &crate::pages::node_editor::tabs::OpenTab) {
    let fp = crate::pages::node_editor::persist::tab_fingerprint(&tab.ctx);
    tab.last_saved_fp.set(fp);
    if tab.dirty.get_untracked() {
        tab.dirty.set(false);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Confirm-delete dialog (используется из template_card)
// ─────────────────────────────────────────────────────────────────────────────

/// Создаёт `Dialog`, привязанный к open-сигналу, который подтверждает
/// удаление шаблона.
pub fn delete_dialog(open: RwSignal<bool>, target_name: String, target_id: String) -> impl Widget {
    let target_id_clone = target_id.clone();
    Dialog::new(tr!("templates.dialog.delete.title", name = target_name))
        .body(tr!("templates.dialog.delete.body"))
        .is_open(open)
        .action(DialogAction::new(tr!("app.cancel"), move || {
            open.set(false);
        }))
        .action(
            DialogAction::new(tr!("app.delete"), move || {
                let app = use_context::<AppCtx>();
                match templates::delete(&target_id_clone, false) {
                    Ok(()) => {
                        app.notifications.success(tr!("templates.notify.deleted"));
                        bump_revision();
                    }
                    Err(e) => {
                        app.notifications.error(tr!("templates.notify.delete_failed", error = e));
                    }
                }
                open.set(false);
            })
            .primary(),
        )
}
