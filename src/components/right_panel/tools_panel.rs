//! Общие UI-секции «Инструменты» и «Скилы» для правого сайдбара.
//!
//! Используются в двух местах:
//! - `llama_control` (llama.cpp-чат) — секции под выбором модели/контролами;
//! - `pages/syn_chat/right_panel` — первая вкладка «Инструменты».
//!
//! Источники данных — `AppCtx.tools.active` и `AppCtx.skills` / `AppCtx.skills_active`.
//! Эти сигналы model-agnostic, поэтому одинаковый UI работает для обеих
//! страниц без параметров.

use syngui::mgui;
use syngui::prelude::*;
use syngui::StyledWidget;

use crate::chat::tools::Tool;
use crate::context::AppCtx;
use crate::icons::*;

// ─────────────────────────────────────────────────────────────────────────────
// Секция «Инструменты» — активные/доступные чипы
// ─────────────────────────────────────────────────────────────────────────────

pub fn tools_section() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let active: Vec<String> = ctx.tools.active.get();
        let all: Vec<&'static Tool> = Tool::all().iter().collect();

        let active_tools: Vec<&'static Tool> = active
            .iter()
            .filter_map(|k| Tool::by_key(k))
            .collect();
        let available_tools: Vec<&'static Tool> = all
            .iter()
            .filter(|t| !active.iter().any(|k| k == t.key))
            .copied()
            .collect();

        let active_block: Box<dyn Widget> = if active_tools.is_empty() {
            Box::new(
                DecoratedBox::new()
                    .class("tools-empty")
                    .child(Text::new("Нет активных инструментов").class("tools-empty-text")),
            )
        } else {
            Box::new(chips_wrap(active_tools, ChipMode::Active))
        };

        let available_block: Box<dyn Widget> = if available_tools.is_empty() {
            Box::new(
                DecoratedBox::new()
                    .class("tools-empty")
                    .child(Text::new("Все доступные инструменты уже активны").class("tools-empty-text")),
            )
        } else {
            Box::new(chips_wrap(available_tools, ChipMode::Available))
        };

        let column = Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(mgui! {
                Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_AUTO_AWESOME).class("tools-section-icon"),
                    Text::new("Инструменты").class("tools-section-title"),
                ]
            })
            .child(
                Text::new("Активные отправляются модели в каждом запросе. Клик по чипу — переключение.")
                    .class("tools-section-hint"),
            )
            .child(Text::new("Активные").class("tools-section-subtitle"))
            .child(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(vec![active_block]),
            )
            .child(Text::new("Доступные").class("tools-section-subtitle"))
            .child(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(vec![available_block]),
            );

        DecoratedBox::new().class("tools-section").child(column)
    }
}

#[derive(Clone, Copy)]
enum ChipMode {
    Active,
    Available,
}

/// Раскладывает чипы по PER_ROW штук в строку. syngui не поддерживает
/// flex-wrap — явная разбивка на Row’ы.
fn chips_wrap(tools: Vec<&'static Tool>, mode: ChipMode) -> impl Widget {
    const PER_ROW: usize = 2;

    let mut rows: Vec<Box<dyn Widget>> = Vec::new();
    for chunk in tools.chunks(PER_ROW) {
        let mut cells: Vec<Box<dyn Widget>> = Vec::new();
        for t in chunk {
            cells.push(make_chip(t, mode));
        }
        rows.push(Box::new(
            Row::new()
                .gap(8.0)
                .main_axis_alignment(MainAxisAlignment::Start)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(cells),
        ));
    }

    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(rows)
}

fn make_chip(tool: &'static Tool, mode: ChipMode) -> Box<dyn Widget> {
    let key_for_click = tool.key.to_string();
    let (class, icon) = match mode {
        ChipMode::Active => ("tools-active-chip", tool.icon),
        ChipMode::Available => ("tools-available-chip", tool.icon),
    };
    let chip = Chip::new(tool.label)
        .icon(icon)
        .on_click(move || toggle_tool(&key_for_click))
        .class(class);
    Box::new(chip)
}

fn toggle_tool(key: &str) {
    let ctx = use_context::<AppCtx>();
    let key = key.to_string();
    ctx.tools.active.update(|list| {
        if let Some(idx) = list.iter().position(|k| k == &key) {
            list.remove(idx);
        } else {
            list.push(key.clone());
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Секция «Скилы» — близнец tools_section, но источник данных — `AppCtx.skills`
// ─────────────────────────────────────────────────────────────────────────────

pub fn skills_section() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let active_keys: Vec<String> = ctx.skills_active.get();
        let all = ctx.skills.get();

        // Скилов вообще нет — компактная подсказка с кнопкой перехода
        // в Settings → Скилы.
        if all.is_empty() {
            let column = Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(mgui! {
                    Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                        Icon::new(MI_PSYCHOLOGY).class("tools-section-icon"),
                        Text::new("Скилы").class("tools-section-title"),
                    ]
                })
                .child(
                    DecoratedBox::new()
                        .class("tools-empty")
                        .child(Text::new("Нет скилов. Создайте на вкладке «Настройки → Скилы».").class("tools-empty-text")),
                )
                .child(navigate_to_skills_btn());
            return DecoratedBox::new().class("tools-section").child(column);
        }

        let active_skills: Vec<crate::skills::Skill> = active_keys
            .iter()
            .filter_map(|k| all.iter().find(|s| &s.id == k).cloned())
            .collect();
        let available_skills: Vec<crate::skills::Skill> = all
            .iter()
            .filter(|s| !active_keys.iter().any(|k| k == &s.id))
            .cloned()
            .collect();

        let active_block: Box<dyn Widget> = if active_skills.is_empty() {
            Box::new(
                DecoratedBox::new()
                    .class("tools-empty")
                    .child(Text::new("Нет активных скилов").class("tools-empty-text")),
            )
        } else {
            Box::new(skill_chips_wrap(active_skills, ChipMode::Active))
        };
        let available_block: Box<dyn Widget> = if available_skills.is_empty() {
            Box::new(
                DecoratedBox::new()
                    .class("tools-empty")
                    .child(Text::new("Все скилы уже активны").class("tools-empty-text")),
            )
        } else {
            Box::new(skill_chips_wrap(available_skills, ChipMode::Available))
        };

        let column = Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(mgui! {
                Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_PSYCHOLOGY).class("tools-section-icon"),
                    Text::new("Скилы").class("tools-section-title"),
                ]
            })
            .child(
                Text::new("Активные доступны tool'у `autoskill`. Клик по чипу — переключение.")
                    .class("tools-section-hint"),
            )
            .child(Text::new("Активные").class("tools-section-subtitle"))
            .child(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(vec![active_block]),
            )
            .child(Text::new("Доступные").class("tools-section-subtitle"))
            .child(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(vec![available_block]),
            );
        DecoratedBox::new().class("tools-section").child(column)
    }
}

/// Раскладка чипов скилов — точная копия `chips_wrap` для tools.
fn skill_chips_wrap(skills: Vec<crate::skills::Skill>, mode: ChipMode) -> impl Widget {
    const PER_ROW: usize = 2;
    let mut rows: Vec<Box<dyn Widget>> = Vec::new();
    for chunk in skills.chunks(PER_ROW) {
        let mut cells: Vec<Box<dyn Widget>> = Vec::new();
        for s in chunk {
            cells.push(make_skill_chip(s.clone(), mode));
        }
        rows.push(Box::new(
            Row::new()
                .gap(8.0)
                .main_axis_alignment(MainAxisAlignment::Start)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(cells),
        ));
    }
    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(rows)
}

fn make_skill_chip(skill: crate::skills::Skill, mode: ChipMode) -> Box<dyn Widget> {
    let key_for_click = skill.id.clone();
    let class = match mode {
        ChipMode::Active => "tools-active-chip skills-chip",
        ChipMode::Available => "tools-available-chip skills-chip",
    };
    let chip = Chip::new(skill.name)
        .icon(MI_PSYCHOLOGY)
        .on_click(move || toggle_skill(&key_for_click))
        .class(class);
    Box::new(chip)
}

fn toggle_skill(key: &str) {
    let ctx = use_context::<AppCtx>();
    let key = key.to_string();
    ctx.skills_active.update(|list| {
        if let Some(idx) = list.iter().position(|k| k == &key) {
            list.remove(idx);
        } else {
            list.push(key.clone());
        }
    });
}

fn navigate_to_skills_btn() -> impl Widget {
    Button::new("Открыть настройки скилов")
        .leading_icon(MI_PSYCHOLOGY)
        .on_click(|| {
            let ctx = use_context::<AppCtx>();
            if ctx.current_route.get_untracked() != "settings" {
                ctx.router.lock().unwrap().navigate("settings");
                ctx.current_route.set("settings".into());
            }
            ctx.settings_router.lock().unwrap().navigate("skills");
            ctx.selected_settings_tab.set("skills".into());
        })
        .class("llama-btn-console")
}
