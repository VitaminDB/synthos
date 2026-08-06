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
                        Text::new("Общие").class("settings-page-title"),
                        Text::new("Основные параметры приложения.").class("settings-page-subtitle"),

                        section("Интерфейс", vec![
                            text_row(MI_CONTACTS, "Отображаемое имя",
                                "Как подписываются исходящие сообщения", g.display_name),
                            dropdown_row(MI_TRANSLATE, "Язык интерфейса",
                                "Перезагрузка не требуется", g.language, vec![
                                    DropdownItem::new("ru", "Русский"),
                                    DropdownItem::new("en", "English"),
                                    DropdownItem::new("de", "Deutsch"),
                                    DropdownItem::new("es", "Español"),
                                ]),
                        ]),

                        section("AI-ассистент", vec![
                            textarea_row(MI_CHAT, "Системный промпт",
                                "Первое сообщение (role=system) в каждом запросе к модели. Задаёт тон и ограничения ассистента.",
                                g.system_prompt),
                            textarea_row(MI_RECORD_VOICE_OVER, "Системный промпт постобработки речи",
                                "Применяется при распознавании голоса — кнопка ⟲ в FAB-окне прогоняет сырую стенограмму через локальную модель с этим промптом и заменяет ею «Отредактированный текст». Пусто = постобработка отключена.",
                                g.voice_refine_prompt),
                        ]),

                        section("Глубина агентов", vec![
                            turns_row(MI_AUTORENEW, "Глубина основного агента",
                                "Сколько подряд tool-вызовов разрешено сделать в одном цикле run_agent. По исчерпании цикл выходит. Больше — длиннее цепочки, выше риск зацикливания.",
                                g.agent_max_turns, 1, 128),
                            turns_row(MI_BOLT, "Глубина субагента",
                                "Сколько tool-вызовов разрешено внутри одного субагента до финального summarize-turn без tools. Не превышает основной лимит на практике, можно ставить меньше.",
                                g.subagent_max_turns, 1, 64),
                        ]),

                        section("Автокомпактификация контекста", vec![
                            switch_row(MI_COMPRESS, "Включить autocompact",
                                "Когда заполнение контекстного окна превышает порог, старые сообщения автоматически сжимаются в краткое system-сообщение. Доступна и ручная кнопка «Compact now» в правой панели.",
                                g.autocompact_enabled),
                            turns_row(MI_TUNE, "Порог срабатывания, %",
                                "При prompt_tokens / n_ctx больше этого процента запускается компактификация. 50–95.",
                                g.autocompact_threshold_percent, 50, 95),
                        ]),

                        section("Окно голосового распознавания", vec![
                            text_row(MI_RECORD_VOICE_OVER, "Семейство шрифта",
                                "Применяется к распознанному тексту в FAB-окне. Пусто = sans-serif.",
                                g.voice_font_family),
                            font_size_row(MI_TUNE, "Размер шрифта",
                                "В логических пикселях. Применяется через MSS-переменную в реальном времени.",
                                g.voice_font_size, 14.0, 40.0),
                        ]),

                        section("Редактор кода", vec![
                            text_row(MI_FONT_DOWNLOAD, "Семейство шрифта",
                                "Применяется к CodeEditor на странице «Код». Пусто = monospace (системный default).",
                                ctx.code_editor_font_family),
                            font_size_row(MI_TUNE, "Размер шрифта",
                                "В логических пикселях. Применяется через MSS-переменную в реальном времени.",
                                ctx.code_editor_font_size, 10.0, 24.0),
                        ]),

                        section("Чат", vec![
                            dropdown_row(MI_TUNE, "Вызовы инструментов",
                                "Что показывать в ленте при работе агента (bash, web_read и др.)",
                                g.tool_display_mode, vec![
                                    DropdownItem::new("full",    "Полный вывод"),
                                    DropdownItem::new("minimal", "Минимальная информация"),
                                    DropdownItem::new("hidden",  "Скрыть"),
                                ]),
                        ]),

                        section("HuggingFace", {
                            let hf = use_context::<HuggingFaceCtx>();
                            vec![
                                text_row(MI_LOCK, "Токен доступа",
                                    "Токен HuggingFace (hf_…) для gated/private моделей (FLUX.1-dev, Llama) и снятия anon-лимита. Создать: huggingface.co/settings/tokens (роль read). Пусто = только публичные.",
                                    hf.token),
                                folder_row(MI_FOLDER, "Каталог моделей",
                                    "Куда сохраняются скачанные веса. Пусто = ~/.local/share/synthos/hf.",
                                    hf.cache_dir),
                                turns_row(MI_CLOUD_DOWNLOAD, "Одновременно файлов",
                                    "Сколько файлов скачивается параллельно. Остальные ждут в очереди.",
                                    hf.concurrent_limit, 1, 8),
                                turns_row(MI_TUNE, "Сегментов на файл",
                                    "Файл режется на N HTTP-Range-сегментов и качается параллельно. Ускоряет крупные шарды.",
                                    hf.segments_per_file, 1, 16),
                                turns_row(MI_SPEED, "Лимит скорости (МБ/с)",
                                    "Общий потолок на все загрузки и их сегменты. 0 = без лимита.",
                                    hf.speed_limit_mbps, 0, 2000),
                                switch_row(MI_FILTER_ALT, "Пропускать onnx/bin/fp32",
                                    "«Скачать всё» не тянет несовместимые/тяжёлые форматы (onnx, openvino, fp32, .bin). Кнопка «Скачать» на файле качает что угодно.",
                                    hf.skip_unwanted_formats),
                                switch_row(MI_FOLDER_ZIP, "Поддержка GGUF",
                                    "Включает поиск и скачивание GGUF-моделей на странице HuggingFace и конвертацию .gguf → .syn прямо из списка файлов. Выключено — GGUF не ищется и не качается.",
                                    hf.gguf_support),
                            ]
                        }),

                        section("Разрешения инструментов", {
                            // Глобальный режим + per-tool override на каждый
                            // зарегистрированный инструмент. Tool::all() — единый
                            // источник правды (chat::tools::descriptor), новый
                            // инструмент в каталоге автоматически появится здесь.
                            let mut rows: Vec<Box<dyn Widget>> = vec![
                                dropdown_row(MI_VERIFIED_USER, "Режим по умолчанию",
                                    "Применяется ко всем инструментам без отдельного override",
                                    g.tool_approval_default, vec![
                                        DropdownItem::new(TOOL_APPROVAL_ASK,    "Спрашивать всегда"),
                                        DropdownItem::new(TOOL_APPROVAL_ALWAYS, "Разрешать без подтверждения"),
                                    ]),
                            ];
                            for tool in Tool::all() {
                                rows.push(tool_override_row(
                                    tool.key, tool.icon, tool.label,
                                    "Переопределяет режим по умолчанию для этого инструмента",
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

fn section(title: &'static str, rows: Vec<Box<dyn Widget>>) -> impl Widget {
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

fn row_frame(
    icon: &'static str,
    title: &'static str,
    desc: &'static str,
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
    title: &'static str,
    desc: &'static str,
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
    title: &'static str,
    desc: &'static str,
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
    title: &'static str,
    desc: &'static str,
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
    title: &'static str,
    desc: &'static str,
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
        .placeholder("Введите системный промпт…")
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
    label: &'static str,
    desc: &'static str,
    overrides: RwSignal<HashMap<String, String>>,
) -> Box<dyn Widget> {
    let initial = overrides
        .get_untracked()
        .get(tool_key)
        .cloned()
        .unwrap_or_else(|| TOOL_APPROVAL_DEFAULT.to_string());

    let items = vec![
        DropdownItem::new(TOOL_APPROVAL_DEFAULT, "По умолчанию"),
        DropdownItem::new(TOOL_APPROVAL_ASK,     "Спрашивать"),
        DropdownItem::new(TOOL_APPROVAL_ALWAYS,  "Разрешать всегда"),
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
    title: &'static str,
    desc: &'static str,
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
fn switch_row(
    icon: &'static str,
    title: &'static str,
    desc: &'static str,
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
    title: &'static str,
    desc: &'static str,
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
