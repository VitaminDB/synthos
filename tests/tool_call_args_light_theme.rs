//! Плашка аргументов tool-вызова не должна быть тёмной на светлой теме.
//!
//! Зачем тест: аргументы рисует `MarkdownView` (JSON в fence), а у него фон
//! код-блока задаётся не `background-color`, а `--md-code-block-bg`. В
//! `.tool-call-args` этой переменной не было, и виджет брал свой дефолт
//! `#1E293B` — на светлой теме внутри светлой карточки («Размышления» →
//! «пишет команду…») висел тёмный прямоугольник, а текст подсветки
//! `InspiredGitHub` на нём был нечитаемым тёмно-серым.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::core::Color;
use syngui::prelude::*;
use syngui::testing::*;
use syngui::widgets::visual::MarkdownView;

/// Карточка live-превью tool-вызова — как её собирает
/// `message_bubble::streaming_tool_preview`: обёртка `.tool-call-args-wrap`
/// с фоном и `MarkdownView` с классом `.tool-call-args` внутри.
fn preview(syntax_theme: &str) -> TestHarness {
    let md = "```json\n{\"action\": \"read\", \"page\": \"MyLife\"}\n```";
    let card = DecoratedBox::new()
        .class("tool-call-card tool-call-streaming")
        .child(
            DecoratedBox::new().class("tool-call-args-wrap").child(
                MarkdownView::new(md)
                    .with_copy_code(false)
                    .with_syntax_theme(syntax_theme)
                    .class("tool-call-args"),
            ),
        );
    let mut harness = TestHarness::new(Box::new(card));
    let engine = harness.apply_mss(synthos::styles::styles());
    harness.apply_styles(&engine);
    harness.layout(720.0, 400.0);
    harness
}

/// Воспринимаемая яркость: тёмная плашка от светлой отличается именно ею.
/// Считаем по **sRGB**-компонентам — `Color` хранит линейные, и в них даже
/// откровенно светлый токен темы `base16-ocean.dark` даёт всего ~0.55.
fn luma(c: Color) -> f32 {
    let [r, g, b] = c.to_srgb_u8();
    let f = |v: u8| v as f32 / 255.0;
    0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

/// Ни один непрозрачный прямоугольник внутри карточки не должен быть
/// тёмным: и сама карточка, и обёртка, и код-блок живут на светлой теме.
#[test]
fn args_block_is_not_dark_on_light_theme() {
    let mut h = preview("InspiredGitHub");
    for cmd in h.paint().commands() {
        if let DrawCommand::Rect { rect, color, .. } = cmd {
            // Прозрачный фон код-блока — норма: подложку даёт обёртка.
            if color.a < 0.2 || rect.size.width < 40.0 || rect.size.height < 10.0 {
                continue;
            }
            assert!(
                luma(color) > 0.5,
                "тёмная плашка {:?} размером {:?} на светлой теме",
                color,
                rect.size
            );
        }
    }
}

/// Текст подсветки на светлой теме читаем: syntect отдаёт тёмные токены,
/// и ложиться они должны на светлую подложку (см. тест выше).
#[test]
fn args_text_is_dark_on_light_theme() {
    let mut h = preview("InspiredGitHub");
    let mut seen = 0usize;
    for cmd in h.paint().commands() {
        if let DrawCommand::Text { text, color, .. } = cmd {
            if text.trim().is_empty() {
                continue;
            }
            seen += 1;
            assert!(
                luma(color) < 0.5,
                "светлый токен {:?} ({:?}) на светлой подложке",
                text,
                color
            );
        }
    }
    assert!(seen > 0, "в display list нет текста аргументов");
}

/// Обратная половина: на тёмной теме подсветка светлая, и прозрачный фон
/// код-блока не «съедает» её — подложку по-прежнему даёт `.tool-call-args-wrap`.
#[test]
fn args_text_is_light_on_dark_syntax_theme() {
    let mut h = preview("base16-ocean.dark");
    let mut light = 0usize;
    for cmd in h.paint().commands() {
        if let DrawCommand::Text { text, color, .. } = cmd {
            if !text.trim().is_empty() && luma(color) > 0.5 {
                light += 1;
            }
        }
    }
    assert!(light > 0, "тёмная тема подсветки не дала ни одного светлого токена");
}
