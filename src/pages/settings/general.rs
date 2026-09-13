//! Подстраница «Общие» — переключатели и текстовые поля из `ctx.general`.
//!
//! Все сигналы живут в `AppCtx::general` (см. `context.rs`); изменения
//! автоматически попадают в `~/.config/synthos/config.json` через
//! `install_config_autosave()`.

use std::collections::HashMap;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::MultilineTextEdit;

use crate::agent::tools::Tool;
use crate::config::{TOOL_APPROVAL_ALWAYS, TOOL_APPROVAL_ASK, TOOL_APPROVAL_DEFAULT};
use crate::context::AppCtx;
use crate::icons::*;
use crate::pages::huggingface::HuggingFaceCtx;

pub fn view() -> impl Widget {
    let ctx = use_context::<AppCtx>();
    let g = ctx.general;

    mgui! {
        ScrollView::new().vertical() => [
            Padding::all(32.0) => [
                DecoratedBox::new().class("settings-page") => [
                    Column::new().gap(24.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                        Text::new(tr!("settings.general.title")).class("settings-page-title"),
                        Text::new(tr!("settings.general.subtitle")).class("settings-page-subtitle"),

                        section(tr!("settings.general.section.interface"), vec![
                            dropdown_row(MI_TRANSLATE, tr!("settings.general.language"),
                                tr!("settings.general.language.desc"), g.language,
                                crate::i18n::language_items()),
                        ]),

                        section(tr!("settings.general.section.assistant"), vec![
                            textarea_row(MI_CHAT, tr!("settings.general.system_prompt"),
                                tr!("settings.general.system_prompt.desc"),
                                g.system_prompt),
                            textarea_row(MI_RECORD_VOICE_OVER, tr!("settings.general.voice_refine_prompt"),
                                tr!("settings.general.voice_refine_prompt.desc"),
                                g.voice_refine_prompt),
                        ]),

                        section(tr!("settings.general.section.agent_depth"), vec![
                            turns_row(MI_AUTORENEW, tr!("settings.general.agent_max_turns"),
                                tr!("settings.general.agent_max_turns.desc"),
                                g.agent_max_turns, 1, 128),
                            turns_row(MI_BOLT, tr!("settings.general.subagent_max_turns"),
                                tr!("settings.general.subagent_max_turns.desc"),
                                g.subagent_max_turns, 1, 64),
                        ]),

                        section(tr!("settings.general.section.autocompact"), vec![
                            switch_row(MI_COMPRESS, tr!("settings.general.autocompact_enabled"),
                                tr!("settings.general.autocompact_enabled.desc"),
                                g.autocompact_enabled),
                            turns_row(MI_TUNE, tr!("settings.general.autocompact_threshold"),
                                tr!("settings.general.autocompact_threshold.desc"),
                                g.autocompact_threshold_percent, 50, 95),
                        ]),

                        section(tr!("settings.general.section.voice_window"), vec![
                            text_row(MI_RECORD_VOICE_OVER, tr!("settings.general.font_family"),
                                tr!("settings.general.voice_font_family.desc"),
                                g.voice_font_family),
                            font_size_row(MI_TUNE, tr!("settings.general.font_size"),
                                tr!("settings.general.voice_font_size.desc"),
                                g.voice_font_size, 14.0, 40.0),
                        ]),

                        section(tr!("settings.general.section.code_editor"), vec![
                            text_row(MI_FONT_DOWNLOAD, tr!("settings.general.font_family"),
                                tr!("settings.general.code_font_family.desc"),
                                ctx.code_editor_font_family),
                            font_size_row(MI_TUNE, tr!("settings.general.font_size"),
                                tr!("settings.general.code_font_size.desc"),
                                ctx.code_editor_font_size, 10.0, 24.0),
                        ]),

                        section(tr!("settings.general.section.chat"), vec![
                            dropdown_row(MI_TUNE, tr!("settings.general.tool_display"),
                                tr!("settings.general.tool_display.desc"),
                                g.tool_display_mode, vec![
                                    DropdownItem::new("full", tr!("settings.general.tool_display.full")),
                                    DropdownItem::new("minimal", tr!("settings.general.tool_display.minimal")),
                                    DropdownItem::new("hidden", tr!("settings.general.tool_display.hidden")),
                                ]),
                        ]),

                        section(tr!("settings.general.section.huggingface"), {
                            let hf = use_context::<HuggingFaceCtx>();
                            vec![
                                text_row(MI_LOCK, tr!("settings.general.hf_token"),
                                    tr!("settings.general.hf_token.desc"),
                                    hf.token),
                                folder_row(MI_FOLDER, tr!("settings.general.hf_cache_dir"),
                                    tr!("settings.general.hf_cache_dir.desc"),
                                    hf.cache_dir),
                                turns_row(MI_CLOUD_DOWNLOAD, tr!("settings.general.hf_concurrent"),
                                    tr!("settings.general.hf_concurrent.desc"),
                                    hf.concurrent_limit, 1, 8),
                                turns_row(MI_TUNE, tr!("settings.general.hf_segments"),
                                    tr!("settings.general.hf_segments.desc"),
                                    hf.segments_per_file, 1, 16),
                                turns_row(MI_SPEED, tr!("settings.general.hf_speed_limit"),
                                    tr!("settings.general.hf_speed_limit.desc"),
                                    hf.speed_limit_mbps, 0, 2000),
                                switch_row(MI_FILTER_ALT, tr!("settings.general.hf_skip_formats"),
                                    tr!("settings.general.hf_skip_formats.desc"),
                                    hf.skip_unwanted_formats),
                                switch_row(MI_FOLDER_ZIP, tr!("settings.general.hf_gguf"),
                                    tr!("settings.general.hf_gguf.desc"),
                                    hf.gguf_support),
                            ]
                        }),

                        section(tr!("settings.general.section.tool_permissions"), {
                            // Глобальный режим + per-tool override на каждый
                            // зарегистрированный инструмент. Tool::all() — единый
                            // источник правды (chat::tools::descriptor), новый
                            // инструмент в каталоге автоматически появится здесь.
                            let mut rows: Vec<Box<dyn Widget>> = vec![
                                dropdown_row(MI_VERIFIED_USER, tr!("settings.general.tool_approval_default"),
                                    tr!("settings.general.tool_approval_default.desc"),
                                    g.tool_approval_default, vec![
                                        DropdownItem::new(TOOL_APPROVAL_ASK, tr!("settings.general.tool_approval.ask_always")),
                                        DropdownItem::new(TOOL_APPROVAL_ALWAYS, tr!("settings.general.tool_approval.allow")),
                                    ]),
                            ];
                            // `autotools` только читает каталог и не спрашивает.
                            for tool in Tool::selectable() {
                                rows.push(tool_override_row(
                                    tool.key, tool.icon, tool.label,
                                    tr!("settings.general.tool_override.desc"),
                                    g.tool_approval_overrides,
                                ));
                            }
                            rows
                        }),
                    ]
                ]
            ]
        ]
    }
}

fn section(title: impl Into<String>, rows: Vec<Box<dyn Widget>>) -> impl Widget {
    let card = DecoratedBox::new().class("settings-card").child(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    );

    Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(title).class("settings-section-title"))
        .child(card)
}

pub(super) fn row_frame(
    icon: &'static str,
    title: impl Into<String>,
    desc: impl Into<String>,
    control: Box<dyn Widget>,
) -> Box<dyn Widget> {
    let inner = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            DecoratedBox::new()
                .class("settings-row-icon-wrap")
                .child(Center::new().child(Icon::new(icon).class("settings-row-icon"))),
        )
        .child(
            DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(Text::new(title).class("settings-row-title"))
                    .child(Text::new(desc).class("settings-row-desc")),
            ),
        )
        .children(vec![control]);

    Box::new(
        DecoratedBox::new().class("settings-row").child(
            Padding::symmetric(24.0, 18.0).child(inner),
        ),
    )
}

fn text_row(
    icon: &'static str,
    title: impl Into<String>,
    desc: impl Into<String>,
    value: RwSignal<String>,
) -> Box<dyn Widget> {
    let initial = value.get_untracked();
    let control: Box<dyn Widget> = Box::new(
        TextField::with_text(initial).on_change(move |s| value.set(s.to_string())),
    );
    row_frame(icon, title, desc, control)
}

fn dropdown_row(
    icon: &'static str,
    title: impl Into<String>,
    desc: impl Into<String>,
    value: RwSignal<String>,
    items: Vec<DropdownItem>,
) -> Box<dyn Widget> {
    let initial = value.get_untracked();
    let control: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(initial)
            .on_change(move |s| value.set(s.to_string()))
            .class("settings-row-dropdown"),
    );
    row_frame(icon, title, desc, control)
}

/// Строка с текстовым полем + кнопкой «обзор» выбора папки
/// (`pick_folder`) — для каталогов вроде HF-кэша.
fn folder_row(
    icon: &'static str,
    title: impl Into<String>,
    desc: impl Into<String>,
    value: RwSignal<String>,
) -> Box<dyn Widget> {
    let initial = value.get_untracked();
    let field = TextField::with_text(initial)
        .placeholder("~/.local/share/synthos/hf")
        .on_change(move |s| value.set(s.to_string()));

    let browse = ToolButton::new(MI_FOLDER_OPEN)
        .on_click(move || {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                value.set(path.display().to_string());
            }
        })
        .class("models-path-browse");

    let control: Box<dyn Widget> = Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![
                Box::new(field) as Box<dyn Widget>,
                Box::new(browse) as Box<dyn Widget>,
            ]),
    );
    row_frame(icon, title, desc, control)
}

/// Строка с многострочным текстовым полем (textarea). В отличие от обычного
/// `text_row`, поле растянуто на всю ширину ниже заголовка — это корректно
/// для длинных промптов, которые иначе сжимаются в узкую колонку справа.
fn textarea_row(
    icon: &'static str,
    title: impl Into<String>,
    desc: impl Into<String>,
    value: RwSignal<String>,
) -> Box<dyn Widget> {
    let initial = value.get_untracked();

    let header = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            DecoratedBox::new()
                .class("settings-row-icon-wrap")
                .child(Center::new().child(Icon::new(icon).class("settings-row-icon"))),
        )
        .child(
            DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(Text::new(title).class("settings-row-title"))
                    .child(Text::new(desc).class("settings-row-desc")),
            ),
        );

    // Сам textarea — на всю ширину карточки, с фиксированным начальным числом
    // строк; `soft_wrap(true)` + `auto_height(true)` дадут органичный рост.
    // Класс ставим напрямую на виджет — MultilineTextEdit сам читает
    // background-color / border-color / color через свой ComputedStyle, обёртка
    // DecoratedBox только добавила бы лишний слой и повторный фон.
    let field = MultilineTextEdit::new()
        .text(initial)
        .placeholder(tr!("settings.general.prompt.placeholder"))
        .rows(4)
        .soft_wrap(true)
        .auto_height(true)
        .on_change(move |s| value.set(s.to_string()));

    let column = Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(header)
        .child(field);

    Box::new(
        DecoratedBox::new()
            .class("settings-row")
            .child(Padding::symmetric(24.0, 18.0).child(column)),
    )
}

/// Строка-override для конкретного инструмента: Dropdown с тремя пунктами,
/// читающий/пишущий ключ в `RwSignal<HashMap>`. При выборе «По умолчанию»
/// ключ удаляется из map (отсутствие ключа = «использовать глобальный режим»),
/// иначе пишется конкретное значение. Сериализация и автосохранение
/// прозрачно подхватываются `install_config_autosave` через подписку на
/// `g.tool_approval_overrides`.
fn tool_override_row(
    tool_key: &'static str,
    icon: &'static str,
    label: impl Into<String>,
    desc: impl Into<String>,
    overrides: RwSignal<HashMap<String, String>>,
) -> Box<dyn Widget> {
    let initial = overrides
        .get_untracked()
        .get(tool_key)
        .cloned()
        .unwrap_or_else(|| TOOL_APPROVAL_DEFAULT.to_string());

    let items = vec![
        DropdownItem::new(TOOL_APPROVAL_DEFAULT, tr!("settings.general.tool_override.default")),
        DropdownItem::new(TOOL_APPROVAL_ASK, tr!("settings.general.tool_override.ask")),
        DropdownItem::new(TOOL_APPROVAL_ALWAYS, tr!("settings.general.tool_override.allow")),
    ];

    let key = tool_key.to_string();
    let control: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(initial)
            .on_change(move |s| {
                let s = s.to_string();
                overrides.update(|m| {
                    if s == TOOL_APPROVAL_DEFAULT {
                        m.remove(&key);
                    } else {
                        m.insert(key.clone(), s);
                    }
                });
            })
            .class("settings-row-dropdown"),
    );
    row_frame(icon, label, desc, control)
}

/// Универсальная строка SpinBox для размера шрифта (px, целые). Параметры
/// `min`/`max` задают допустимые границы и одновременно clamp'ят значение
/// при ручном вводе. Использовалось как `voice_font_size_row` (14..40);
/// теперь обобщено и для CodeEditor (10..24).
fn font_size_row(
    icon: &'static str,
    title: impl Into<String>,
    desc: impl Into<String>,
    value: RwSignal<f32>,
    min: f32,
    max: f32,
) -> Box<dyn Widget> {
    let initial = value.get_untracked();
    let control: Box<dyn Widget> = Box::new(
        SpinBox::new()
            .value(initial as f64)
            .range(min as f64, max as f64)
            .step(1.0)
            .decimal_places(0)
            .on_change(move |v| value.set(v.clamp(min as f64, max as f64) as f32))
            .width(140.0),
    );
    row_frame(icon, title, desc, control)
}

/// SpinBox-строка для целочисленного лимита turn'ов агента/субагента.
/// Разделена от `font_size_row` (там f32, дробей мы не хотим) и `port_row`
/// (тот ограничен u16 1..=65535 — для нас бессмысленно).
pub(super) fn switch_row(
    icon: &'static str,
    title: impl Into<String>,
    desc: impl Into<String>,
    value: RwSignal<bool>,
) -> Box<dyn Widget> {
    let initial = value.get_untracked();
    let control: Box<dyn Widget> = Box::new(
        Toggle::with_state(initial).on_change(move |v| value.set(v)),
    );
    row_frame(icon, title, desc, control)
}

fn turns_row(
    icon: &'static str,
    title: impl Into<String>,
    desc: impl Into<String>,
    value: RwSignal<u32>,
    min: u32,
    max: u32,
) -> Box<dyn Widget> {
    let initial = value.get_untracked();
    let control: Box<dyn Widget> = Box::new(
        SpinBox::new()
            .value(initial as f64)
            .range(min as f64, max as f64)
            .step(1.0)
            .decimal_places(0)
            .on_change(move |v| value.set(v.clamp(min as f64, max as f64) as u32))
            .width(140.0),
    );
    row_frame(icon, title, desc, control)
}
