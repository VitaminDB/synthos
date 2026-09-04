//! Свёрнутая цепочка tool-вызовов: счётчики исхода и оформление шагов.
//!
//! Зачем тест: и то, и другое держится на потомковых селекторах
//! (`.tool-group-count-ok .tool-group-count-text`,
//! `.tool-group-children .tool-call-card`). Если такой селектор перестанет
//! матчиться или класс переименуют, ошибка выйдет не сборкой, а тихим
//! «пилюли серые, внутри 14 рамок подряд» — увидеть это можно только глазами
//! в GUI. Здесь то же самое проверяется на разрешённых MSS-полях.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::core::Color;
use syngui::prelude::*;
use syngui::testing::TestHarness;

use synthos::icons::{MI_CHECK, MI_REPORT, MI_TERMINAL};

/// Пилюля-счётчик — как её собирает `tool_group::count_pill`.
fn pill(icon: &'static str, n: &str, tone: &str) -> Box<dyn Widget> {
    Box::new(
        DecoratedBox::new()
            .class(format!("tool-group-count {tone}"))
            .child(
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(vec![
                        Box::new(Icon::new(icon).class("tool-group-count-icon")) as Box<dyn Widget>,
                        Box::new(Text::new(n).class("tool-group-count-text")),
                    ]),
            ),
    )
}

/// Карточка группы «6 успешных, 1 с ошибкой» с раскрытой цепочкой внутри.
fn chain(card_class: &str) -> TestHarness {
    let header = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![
            Box::new(
                DecoratedBox::new()
                    .class("tool-group-icon-wrap")
                    .child(Center::new().child(Icon::new(MI_TERMINAL).class("tool-group-icon"))),
            ) as Box<dyn Widget>,
            Box::new(Text::new("Веб").class("tool-group-name")),
            pill(MI_CHECK, "6", "tool-group-count-ok"),
            pill(MI_REPORT, "1", "tool-group-count-error"),
        ]);
    let body = DecoratedBox::new().class("tool-group-children").child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                Box::new(
                    DecoratedBox::new()
                        .class("tool-call-card")
                        .child(Text::new("{\"query\":\"unlock bootloader\"}")),
                ) as Box<dyn Widget>,
                Box::new(
                    DecoratedBox::new()
                        .class("tool-result-card tool-result-card-error")
                        .child(Text::new("HTTP 429")),
                ),
            ]),
    );
    let mut harness = TestHarness::new(Box::new(
        DecoratedBox::new().class(card_class).child(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![Box::new(header) as Box<dyn Widget>, Box::new(body)]),
        ),
    ));
    let engine = harness.apply_mss(synthos::styles::styles());
    harness.apply_styles(&engine);
    harness.layout(720.0, 400.0);
    harness
}

fn near(a: Color, b: Color) -> bool {
    (a.r - b.r).abs() < 0.01 && (a.g - b.g).abs() < 0.01 && (a.b - b.b).abs() < 0.01
}

/// Цвет иконки счётчика. Проверяем именно её: `Text` разрешённые MSS-поля
/// наружу не отдаёт (`Element::mss` у него `None`), а красится иконка тем же
/// потомковым правилом, что и число рядом.
#[test]
fn outcome_pills_are_green_and_red() {
    let h = chain("tool-group-card tool-group-card-mixed");
    let ids = h.find_by_class("tool-group-count-icon");
    assert_eq!(ids.len(), 2, "ожидали две пилюли-счётчика");
    let color = |i: usize| {
        h.element_mss(ids[i])
            .and_then(|f| f.color)
            .expect("у иконки счётчика не разрешился color")
    };
    let ok = color(0);
    assert!(near(ok, Color::from_hex("#22C55E")), "успешный счётчик не зелёный: {ok:?}");
    let err = color(1);
    assert!(near(err, Color::from_hex("#E55353")), "счётчик ошибок не красный: {err:?}");
}

/// Пилюли — это пилюли: скруглённый фон под каждым счётчиком.
#[test]
fn outcome_pills_have_soft_backgrounds() {
    let h = chain("tool-group-card tool-group-card-mixed");
    for class in ["tool-group-count-ok", "tool-group-count-error"] {
        let ids = h.find_by_class(class);
        assert!(!ids.is_empty(), "не нашли .{class}");
        let f = h.element_mss(ids[0]).expect("стили пилюли");
        let bg = f.background_color.expect("у пилюли нет фона");
        assert!(bg.a > 0.05 && bg.a < 0.5, ".{class}: фон должен быть мягкой подложкой, а не заливкой");
        assert!(f.border_radius.is_some(), ".{class}: пилюля без скругления");
    }
}

#[test]
fn chain_steps_lose_their_own_frames() {
    let h = chain("tool-group-card tool-group-card-mixed");
    for class in ["tool-call-card", "tool-result-card"] {
        let ids = h.find_by_class(class);
        assert!(!ids.is_empty(), "не нашли .{class}");
        let f = h.element_mss(ids[0]).expect("стили шага");
        assert_eq!(f.border_width, Some(0.0), ".{class} внутри цепочки без рамки");
        assert!(
            f.background_color.map(|c| c.a).unwrap_or(1.0) < 0.1,
            ".{class} внутри цепочки почти прозрачна"
        );
    }
}

/// Смешанный исход не красит карточку целиком — заливку получает только
/// цепочка, упавшая полностью.
#[test]
fn mixed_chain_keeps_neutral_card() {
    let mixed = chain("tool-group-card tool-group-card-mixed");
    let ids = mixed.find_by_class("tool-group-card");
    let bg = mixed.element_mss(ids[0]).and_then(|f| f.background_color);
    let panel = Color::from_hex("#FFFFFF");
    assert!(
        bg.map(|c| near(c, panel)).unwrap_or(false),
        "смешанная цепочка не должна заливаться красным: {bg:?}"
    );

    let failed = chain("tool-group-card tool-group-card-with-error");
    let ids = failed.find_by_class("tool-group-card");
    let bg = failed
        .element_mss(ids[0])
        .and_then(|f| f.background_color)
        .expect("фон упавшей цепочки");
    assert!(bg.r > bg.g && bg.r > bg.b, "упавшая цепочка должна быть красноватой: {bg:?}");
}
