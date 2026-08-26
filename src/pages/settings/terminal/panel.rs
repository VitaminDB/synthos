//! Общая панель настроек шрифта VTE-терминала.
//!
//! Используется в двух местах:
//! 1. Settings → секция «Терминал» (главное место).
//! 2. Gear-popover в правом верхнем углу самого терминала на странице
//!    «Редактор кода» — даёт быстрый доступ без перехода в настройки.
//!
//! Бьёт по сигналам `AppCtx.terminal_font_family` / `terminal_font_size`,
//! которые сохраняются в `~/.config/synthos/config.json` через общий
//! autosave-effect в `lib.rs`. Виджет [`syngui::widgets::Terminal`]
//! подхватывает изменения через `Reactive`-обёртку без рестарта PTY
//! (Element::update обновляет config + cell-метрики).

use syngui::prelude::*;

use crate::context::AppCtx;
use crate::icons::*;

/// Спец-значение Dropdown'а: «Системный шрифт по умолчанию».
/// На диск пишется как пустая строка (`String::new()`); резолвится
/// font-kit'ом в DejaVu Sans Mono / Menlo / Consolas в зависимости от ОС.
const DEFAULT_FAMILY_KEY: &str = "__default__";

/// Минимально/максимально допустимый размер шрифта терминала в px.
/// Терминал клампит снизу до 6.0 (см. `Terminal::font_size`); сверху
/// 32 — практический предел читаемости и плотности cell-сетки.
const MIN_FONT_SIZE: f32 = 8.0;
const MAX_FONT_SIZE: f32 = 32.0;

/// Корневой виджет панели — карточка с заголовком, контролами и preview.
pub fn terminal_font_panel() -> impl Widget {
    DecoratedBox::new()
        .class("terminal-settings-panel")
        .child(
            Column::new()
                .gap(20.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![
                    Box::new(section_card(
                        tr!("settings.terminal.section.font"),
                        vec![family_row(), size_row()],
                    )) as Box<dyn Widget>,
                    preview_card(),
                ]),
        )
}

// ─── строки конфигурации ────────────────────────────────────────────────────

fn family_row() -> Box<dyn Widget> {
    let control = DecoratedBox::new()
        .class("terminal-font-control")
        .child(move || {
            let ctx = use_context::<AppCtx>();
            let current = ctx.terminal_font_family.get();

            // Список собирается лениво и кэшируется в font_discovery (OnceLock),
            // повторное открытие настроек не платит цену enum'а.
            let monospace = syngui::text::list_monospace_families();

            let mut items = Vec::with_capacity(monospace.len() + 1);
            items.push(DropdownItem::new(
                DEFAULT_FAMILY_KEY,
                tr!("settings.terminal.family.default"),
            ));
            for name in monospace {
                items.push(DropdownItem::new(name.clone(), name.clone()));
            }

            let selected_key = if current.is_empty() {
                DEFAULT_FAMILY_KEY.to_string()
            } else {
                current.clone()
            };

            Dropdown::with_items(items)
                .selected(selected_key)
                .on_change(|s| {
                    let ctx = use_context::<AppCtx>();
                    let normalized = if s == DEFAULT_FAMILY_KEY {
                        String::new()
                    } else {
                        s.to_string()
                    };
                    ctx.terminal_font_family.set(normalized);
                })
                .class("terminal-font-dropdown")
        });

    row_frame(
        MI_TUNE,
        tr!("settings.terminal.family"),
        tr!("settings.terminal.family.desc"),
        Box::new(control),
    )
}

fn size_row() -> Box<dyn Widget> {
    // Slider не имеет intrinsic-ширины и берёт всю выданную родителем
    // (см. SliderElement::layout — `width.unwrap_or(constraints.max_width)`).
    // Поэтому в горизонтальной Row он схлопывается до ~0px, и трек не
    // зацепить мышью. Раскладываем вертикально: title+value сверху,
    // описание, Slider внизу на всю ширину grow-блока.
    let body = DecoratedBox::new().class("grow").child(move || {
        let ctx = use_context::<AppCtx>();
        let value = ctx.terminal_font_size.get();

        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(
                        DecoratedBox::new()
                            .class("grow")
                            .child(Text::new(tr!("settings.terminal.size")).class("settings-row-title")),
                    )
                    .child(
                        Text::new(tr!("settings.terminal.size.px", n = value as i32))
                            .class("terminal-font-size-value"),
                    ),
            )
            .child(
                Text::new(
                    tr!("settings.terminal.size.desc"),
                )
                .class("settings-row-desc"),
            )
            .child(
                DecoratedBox::new().class("terminal-font-slider-wrap").child(
                    Slider::new()
                        .range(MIN_FONT_SIZE, MAX_FONT_SIZE)
                        .step(1.0)
                        .value(value)
                        .on_change(|v| {
                            let ctx = use_context::<AppCtx>();
                            // Округляем до целого px — терминалу дробные не
                            // дают визуального выигрыша, только дрожание
                            // cell-сетки.
                            ctx.terminal_font_size
                                .set(v.round().clamp(MIN_FONT_SIZE, MAX_FONT_SIZE));
                        })
                        .class("terminal-font-slider"),
                ),
            )
    });

    let inner = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(
            DecoratedBox::new()
                .class("settings-row-icon-wrap")
                .child(Center::new().child(Icon::new(MI_TUNE).class("settings-row-icon"))),
        )
        .child(body);

    Box::new(
        DecoratedBox::new()
            .class("settings-row")
            .child(Padding::symmetric(24.0, 18.0).child(inner)),
    )
}

// ─── live preview ───────────────────────────────────────────────────────────

fn preview_card() -> Box<dyn Widget> {
    // Sample-блок с моноширинным текстом — иллюстрирует формат cell-сетки
    // терминала. Используется generic `monospace` (`.terminal-preview-line`
    // в MSS) — Text-виджет syngui не принимает font-family через builder,
    // и MSS-переменные не пере-резолвятся в runtime без перезагрузки
    // stylesheet. Реальный выбранный шрифт пользователь увидит на самом
    // терминале (страница «Редактор кода») и в подписи ниже.
    let sample = Column::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(Text::new("$ cargo run --release").class("terminal-preview-line"))
        .child(
            Text::new("   Compiling synthos v0.1.0 (1 file)")
                .class("terminal-preview-line"),
        )
        .child(Text::new("   Finished in 12.34s").class("terminal-preview-line"))
        .child(
            Text::new("→ 0xDEADBEEF | 0123456789").class("terminal-preview-line"),
        );

    // Подпись текущих значений — реактивно отражает выбранные шрифт и
    // размер, чтобы пользователь не сомневался, какие настройки активны.
    let caption = DecoratedBox::new()
        .class("terminal-preview-caption-wrap")
        .child(move || {
            let ctx = use_context::<AppCtx>();
            let family = ctx.terminal_font_family.get();
            let size = ctx.terminal_font_size.get();
            let label = if family.is_empty() {
                tr!("settings.terminal.preview.caption", family = tr!("settings.terminal.family.default"), size = size as i32)
            } else {
                tr!("settings.terminal.preview.caption", family = family, size = size as i32)
            };
            Text::new(label).class("terminal-preview-caption")
        });

    let body = DecoratedBox::new()
        .class("terminal-preview-card")
        .child(Padding::all(16.0).child(
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(sample)
                .child(caption),
        ));

    Box::new(
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new(tr!("settings.terminal.preview")).class("settings-section-title"))
            .child(body),
    )
}

// ─── локальные хелперы (упрощённые копии из audio_models/mod.rs) ─────────────
// TODO: вынести row_frame/section_card в общий settings::common — сейчас они
// дублируются в general.rs / models/mod.rs / audio_models/mod.rs / здесь.

fn row_frame(
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
                    .child(Text::new(title.into()).class("settings-row-title"))
                    .child(Text::new(desc.into()).class("settings-row-desc")),
            ),
        )
        .children(vec![control]);

    Box::new(
        DecoratedBox::new()
            .class("settings-row")
            .child(Padding::symmetric(24.0, 18.0).child(inner)),
    )
}

fn section_card(title: impl Into<String>, rows: Vec<Box<dyn Widget>>) -> impl Widget {
    let card = DecoratedBox::new().class("settings-card").child(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    );
    Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(title.into()).class("settings-section-title"))
        .child(card)
}
