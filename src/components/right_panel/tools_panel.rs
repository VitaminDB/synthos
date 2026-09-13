//! Общие UI-секции «Инструменты», «Autotools» и «Скилы» для правого сайдбара.
//!
//! Используются в двух местах:
//! - `llama_control` (llama.cpp-чат) — секции под выбором модели/контролами;
//! - `pages/syn_chat/right_panel` — первая вкладка «Инструменты».
//!
//! Источники данных — `AppCtx.tools.active` / `AppCtx.tools.auto` и
//! `AppCtx.skills` / `AppCtx.skills_active`. Эти сигналы model-agnostic,
//! поэтому одинаковый UI работает для обеих страниц без параметров.

use syngui::mgui;
use syngui::prelude::*;
use syngui::StyledWidget;

use crate::agent::tools::Tool;
use crate::context::AppCtx;
use crate::icons::*;

// ─────────────────────────────────────────────────────────────────────────────
// Секция «Инструменты» — активные/доступные чипы
// ─────────────────────────────────────────────────────────────────────────────

pub fn tools_section() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let active: Vec<String> = ctx.tools.active.get();

        let active_tools: Vec<&'static Tool> = active
            .iter()
            .filter_map(|k| Tool::by_key(k))
            .filter(|t| !t.is_implicit())
            .collect();
        let available_tools: Vec<&'static Tool> = Tool::selectable()
            .filter(|t| !active.iter().any(|k| k == t.key))
            .collect();

        let active_block: Box<dyn Widget> = if active_tools.is_empty() {
            empty_block(tr!("chat.right_panel.tools.empty_active"))
        } else {
            Box::new(chips_wrap(active_tools, ChipMode::Active, toggle_tool))
        };
        let available_block: Box<dyn Widget> = if available_tools.is_empty() {
            empty_block(tr!("chat.right_panel.tools.all_active"))
        } else {
            Box::new(chips_wrap(available_tools, ChipMode::Available, toggle_tool))
        };

        section(
            MI_AUTO_AWESOME,
            tr!("chat.right_panel.tools.title"),
            tr!("chat.right_panel.tools.hint"),
            tr!("chat.right_panel.active"),
            active_block,
            available_block,
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Секция «Autotools» — пул инструментов, которые модель подгружает сама
// ─────────────────────────────────────────────────────────────────────────────

/// Близнец секции «Скилы»: сверху инструменты пула, ниже — все остальные.
/// Клик по доступному кладёт инструмент в пул и снимает его с активных, клик
/// по инструменту в пуле — убирает его оттуда.
pub fn autotools_section() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let auto: Vec<String> = ctx.tools.auto.get();
        let active: Vec<String> = ctx.tools.active.get();

        let pooled = crate::agent::tools::autotools::pool(&active, &auto);
        let available: Vec<&'static Tool> = Tool::selectable()
            .filter(|t| !pooled.iter().any(|p| p.key == t.key))
            .collect();

        let pooled_block: Box<dyn Widget> = if pooled.is_empty() {
            empty_block(tr!("chat.right_panel.autotools.empty"))
        } else {
            Box::new(chips_wrap(pooled, ChipMode::Active, toggle_pool))
        };
        let available_block: Box<dyn Widget> = if available.is_empty() {
            empty_block(tr!("chat.right_panel.autotools.all_pooled"))
        } else {
            Box::new(chips_wrap(available, ChipMode::Available, toggle_pool))
        };

        section(
            MI_HANDYMAN,
            tr!("chat.right_panel.autotools.title"),
            tr!("chat.right_panel.autotools.hint"),
            tr!("chat.right_panel.autotools.pooled"),
            pooled_block,
            available_block,
        )
    }
}

/// Каркас секции: заголовок с иконкой, подсказка, включённые чипы под
/// `first_label` и остальные под «Доступные».
fn section(
    icon: &'static str,
    title: String,
    hint: String,
    first_label: String,
    first: Box<dyn Widget>,
    rest: Box<dyn Widget>,
) -> StyledWidget<DecoratedBox> {
    let column = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(mgui! {
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(icon).class("tools-section-icon"),
                Text::new(title).class("tools-section-title"),
            ]
        })
        .child(Text::new(hint).class("tools-section-hint"))
        .child(Text::new(first_label).class("tools-section-subtitle"))
        .child(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![first]),
        )
        .child(Text::new(tr!("chat.right_panel.available")).class("tools-section-subtitle"))
        .child(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![rest]),
        );

    DecoratedBox::new().class("tools-section").child(column)
}

fn empty_block(text: String) -> Box<dyn Widget> {
    Box::new(
        DecoratedBox::new()
            .class("tools-empty")
            .child(Text::new(text).class("tools-empty-text")),
    )
}

#[derive(Clone, Copy)]
enum ChipMode {
    Active,
    Available,
}

/// Раскладывает чипы по PER_ROW штук в строку. syngui не поддерживает
/// flex-wrap — явная разбивка на Row’ы.
fn chips_wrap(tools: Vec<&'static Tool>, mode: ChipMode, on_toggle: fn(&str)) -> impl Widget {
    const PER_ROW: usize = 2;

    let mut rows: Vec<Box<dyn Widget>> = Vec::new();
    for chunk in tools.chunks(PER_ROW) {
        let mut cells: Vec<Box<dyn Widget>> = Vec::new();
        for t in chunk {
            cells.push(make_chip(t, mode, on_toggle));
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

fn make_chip(tool: &'static Tool, mode: ChipMode, on_toggle: fn(&str)) -> Box<dyn Widget> {
    let key_for_click = tool.key.to_string();
    let (class, icon) = match mode {
        ChipMode::Active => ("tools-active-chip", tool.icon),
        ChipMode::Available => ("tools-available-chip", tool.icon),
    };
    let chip = Chip::new(crate::i18n::tool_label(tool))
        .icon(icon)
        .on_click(move || on_toggle(&key_for_click))
        .class(class);
    Box::new(chip)
}

fn toggle_tool(key: &str) {
    use_context::<AppCtx>().tools.toggle_active(key);
}

fn toggle_pool(key: &str) {
    use_context::<AppCtx>().tools.toggle_pool(key);
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
                        Text::new(tr!("chat.right_panel.skills.title")).class("tools-section-title"),
                    ]
                })
                .child(
                    DecoratedBox::new()
                        .class("tools-empty")
                        .child(Text::new(tr!("chat.right_panel.skills.none")).class("tools-empty-text")),
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
            empty_block(tr!("chat.right_panel.skills.empty_active"))
        } else {
            Box::new(skill_chips_wrap(active_skills, ChipMode::Active))
        };
        let available_block: Box<dyn Widget> = if available_skills.is_empty() {
            empty_block(tr!("chat.right_panel.skills.all_active"))
        } else {
            Box::new(skill_chips_wrap(available_skills, ChipMode::Available))
        };

        section(
            MI_PSYCHOLOGY,
            tr!("chat.right_panel.skills.title"),
            tr!("chat.right_panel.skills.hint"),
            tr!("chat.right_panel.active"),
            active_block,
            available_block,
        )
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
    Button::new(tr!("chat.right_panel.skills.open_settings"))
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
