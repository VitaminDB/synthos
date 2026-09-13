//! Карточки «Thinking» и «Sampling» правой панели чата: переключатель
//! «По умолчанию / Свои» в цветах темы, пикеры на всю ширину карточки,
//! без модели — подсказка вместо слайдеров.
//!
//! Зачем тест: у `SegmentedButton` не было MSS, и он рисовал встроенные
//! дефолты — белую подложку невыбранного сегмента. На тёмной теме это белая
//! плашка посреди панели, а подпись «Свои» не читалась (скриншоты
//! пользователя 13.09.2026). Пикеры пресета и глубины размышлений без
//! `width: 100%` ужимались до своего текста.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use std::sync::Arc;

use synaptix::facade::llm::{ReasoningLevels, ReasoningVar, SamplingPreset, SamplingProfile};
use syngui::core::Color;
use syngui::prelude::*;
use syngui::testing::*;

use synthos::pages::settings::theme_data::{default_dark_theme, default_theme, SynthosTheme};
use synthos::pages::syn_chat::right_panel;
use synthos::syn_chat::params::SamplingMode;
use synthos::syn_chat::{SynChatCtx, SynModelRegistry};

const WIDTH: f32 = 300.0;
/// `.sampling-card { padding: 10px }`.
const INNER: f32 = WIDTH - 20.0;

/// Воспринимаемая яркость по sRGB — `Color` хранит линейные компоненты.
fn luma(c: Color) -> f32 {
    let [r, g, b] = c.to_srgb_u8();
    let f = |v: u8| v as f32 / 255.0;
    0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

/// Профиль Qwen3.8: два пресета и три уровня размышлений.
fn qwen38_profile() -> SamplingProfile {
    let preset = |id, thinking, temperature, top_p, presence_penalty| SamplingPreset {
        id,
        thinking: Some(thinking),
        temperature,
        top_p,
        top_k: 20,
        min_p: 0.0,
        presence_penalty,
        repetition_penalty: 1.0,
    };
    SamplingProfile {
        presets: vec![
            preset("thinking", true, 1.0, 0.95, 0.0),
            preset("instruct", false, 0.7, 0.8, 1.5),
        ],
        reasoning: Some(ReasoningLevels {
            var: ReasoningVar::Effort,
            levels: vec!["low".into(), "medium".into(), "xhigh".into()],
            default: "xhigh".into(),
        }),
    }
}

fn render(widget: Box<dyn Widget>, theme: &SynthosTheme) -> TestHarness {
    let mut h = TestHarness::new(widget);
    let mss = format!("{}\n{}", synthos::styles::styles(), theme.to_mss());
    let engine = h.apply_mss(&mss);
    h.rebuild();
    h.apply_styles(&engine);
    h.layout(WIDTH, 1400.0);
    h
}

fn texts(h: &mut TestHarness) -> Vec<(String, Color)> {
    h.paint()
        .commands()
        .into_iter()
        .filter_map(|cmd| match cmd {
            DrawCommand::Text { text, color, .. } => Some((text.to_string(), color)),
            _ => None,
        })
        .collect()
}

/// Один тест на всё: контексты и сигналы общие, а сценарии переключают их
/// по очереди.
#[test]
fn sampling_and_thinking_cards_follow_theme_and_logic() {
    let home = std::env::temp_dir().join(format!("synthos-sampling-card-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);
    syngui::i18n::register_catalogs(&synthos::i18n::CATALOGS);
    syngui::i18n::set_language(syngui::i18n::Lang::new("ru"));

    provide_context(SynChatCtx::new());
    provide_context(SynModelRegistry::new());
    let ctx = use_context::<SynChatCtx>();
    let reg = use_context::<SynModelRegistry>();
    ctx.cards.sampling.set(true);
    reg.sampling.set(Some(Arc::new(qwen38_profile())));

    // ── Переключатель в цветах темы ─────────────────────────────────────
    for (theme, dark) in [(default_theme(), false), (default_dark_theme(), true)] {
        let mut h = render(Box::new(right_panel::sampling_card()), &theme);
        let switches = h.find_by_class("sampling-mode-switch");
        assert_eq!(switches.len(), 1, "один переключатель режима");
        let sw = h.element_bounds(switches[0]);
        assert!((sw.size.width - INNER).abs() < 1.0, "переключатель на всю ширину: {:?}", sw.size);

        let cmds = h.paint().commands().into_iter().collect::<Vec<_>>();
        // Режим «По умолчанию» выбран — сегмент «Свои» справа невыбранный.
        let own_bg = cmds
            .iter()
            .filter_map(|cmd| match cmd {
                DrawCommand::Rect { rect, color, .. }
                    if color.a > 0.2
                        && rect.size.width > 10.0
                        && rect.origin.x > sw.origin.x + 2.0
                        && rect.origin.y >= sw.origin.y - 0.5
                        && rect.origin.y + rect.size.height <= sw.origin.y + sw.size.height + 0.5 =>
                {
                    Some(*color)
                }
                _ => None,
            })
            .last()
            .expect("подложка невыбранного сегмента");
        let own_text = cmds
            .iter()
            .find_map(|cmd| match cmd {
                DrawCommand::Text { text, color, .. } if text.as_ref() as &str == "Свои" => Some(*color),
                _ => None,
            })
            .expect("подпись «Свои»");
        if dark {
            assert!(luma(own_bg) < 0.5, "на тёмной теме светлая подложка сегмента: {own_bg:?}");
        } else {
            assert!(luma(own_bg) > 0.5, "на светлой теме тёмная подложка сегмента: {own_bg:?}");
        }
        assert!(
            (luma(own_text) - luma(own_bg)).abs() > 0.4,
            "подпись «Свои» не читается (dark={dark}): текст {own_text:?} на {own_bg:?}"
        );
    }

    let light = default_theme();

    // ── Пикеры на всю ширину карточки ──────────────────────────────────
    let h = render(Box::new(right_panel::sampling_card()), &light);
    let pickers = h.find_by_class("sampling-preset-picker");
    assert_eq!(pickers.len(), 1, "пресет модели");
    let b = h.element_bounds(pickers[0]);
    assert!((b.size.width - INNER).abs() < 1.0, "пикер пресета на всю ширину: {:?}", b.size);

    let h = render(Box::new(right_panel::thinking_card()), &light);
    let pickers = h.find_by_class("sampling-preset-picker");
    assert_eq!(pickers.len(), 1, "глубина размышлений у модели с уровнями");
    let b = h.element_bounds(pickers[0]);
    assert!((b.size.width - INNER).abs() < 1.0, "пикер глубины на всю ширину: {:?}", b.size);

    // Без размышлений глубину не выбирают.
    ctx.params.update(|q| q.enable_thinking = false);
    let h = render(Box::new(right_panel::thinking_card()), &light);
    assert!(h.find_by_class("sampling-preset-picker").is_empty(), "глубина при выключенных размышлениях");
    ctx.params.update(|q| q.enable_thinking = true);

    // ── Логика режимов ──────────────────────────────────────────────────
    // По умолчанию: семь слайдеров со значениями пресета + repeat_last_n текстом.
    let mut h = render(Box::new(right_panel::sampling_card()), &light);
    assert_eq!(h.find_by_class("sampling-value").len(), 8);
    let t = texts(&mut h);
    assert!(t.iter().any(|(s, _)| s == "Пресет модели"), "подпись пресета в режиме по умолчанию");
    assert!(t.iter().any(|(s, _)| s == "1.00"), "temperature пресета Thinking");

    // Свои: SpinBox вместо текста, подпись «взять значения».
    ctx.params.update(|q| q.mode = Some(SamplingMode::Custom));
    let mut h = render(Box::new(right_panel::sampling_card()), &light);
    assert_eq!(h.find_by_class("sampling-value").len(), 7);
    let t = texts(&mut h);
    assert!(t.iter().any(|(s, _)| s == "Взять значения из пресета"), "подпись в режиме «Свои»");

    // По умолчанию без модели: значений нет — только подсказка.
    ctx.params.update(|q| q.mode = Some(SamplingMode::Default));
    reg.sampling.set(None);
    let h = render(Box::new(right_panel::sampling_card()), &light);
    assert_eq!(h.find_by_class("sampling-hint").len(), 1, "подсказка «загрузите модель»");
    assert!(h.find_by_class("sampling-value").is_empty(), "слайдеры без модели");
    assert!(h.find_by_class("sampling-preset-picker").is_empty());
}
