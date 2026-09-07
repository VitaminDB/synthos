//! Карточка «Система» правой панели: кнопки библиотеки промптов помещаются
//! в узкую панель, дропдаун пресетов — на всю ширину карточки.
//!
//! Зачем тест: шапка карточки — `Row` с заголовком слева и четырьмя
//! `ToolButton` справа. `measure_row` меряет не-flex детей с бесконечной
//! шириной, так что длинный заголовок или лишняя кнопка выталкивают
//! правую группу за край карточки — и кнопку «открыть в окне» нельзя
//! нажать ровно там, где она нужна (панель сужена до минимума).
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::prelude::*;
use syngui::testing::TestHarness;

use synthos::pages::syn_chat::right_panel;
use synthos::syn_chat::SynChatCtx;

/// Минимальная ширина правой панели (`Pane::new(.., 240.0, ..)` в
/// `pages::syn_chat::view`) минус её внутренние отступы.
const NARROW: f32 = 224.0;

fn settle(harness: &mut TestHarness, engine: &syngui::mss::StyleEngine, width: f32) {
    harness.rebuild();
    harness.apply_styles(engine);
    harness.layout(width, 600.0);
}

fn right_edge(harness: &TestHarness, id: ElementId) -> f32 {
    let b = harness.element_bounds(id);
    b.origin.x + b.size.width
}

#[test]
fn actions_fit_in_narrow_panel_and_picker_spans_card() {
    let home = std::env::temp_dir().join(format!("synthos-prompt-card-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);

    // Без каталогов `tr!` отдаёт сами ключи — заголовок «СИСТЕМА» стал бы
    // длинным `chat.right.system.title`, и тест мерил бы не то, что видит
    // пользователь.
    syngui::i18n::register_catalogs(&synthos::i18n::CATALOGS);
    syngui::i18n::set_language(syngui::i18n::Lang::new("ru"));

    provide_context(SynChatCtx::new());
    let ctx = use_context::<SynChatCtx>();
    // Длинное имя пресета не должно раздвигать карточку.
    let id = ctx.prompt_active.get_untracked();
    synthos::syn_chat::prompt_presets::rename(
        &ctx,
        &id,
        "Очень длинное название пресета, которое точно не помещается в одну строку панели",
    );

    for width in [NARROW, 360.0] {
        let mut harness = TestHarness::new(Box::new(right_panel::system_prompt_card()));
        let engine = harness.apply_mss(synthos::styles::styles());
        settle(&mut harness, &engine, width);

        let cards = harness.find_by_class("sampling-card");
        assert_eq!(cards.len(), 1, "одна карточка");
        let card = harness.element_bounds(cards[0]);
        let card_right = card.origin.x + card.size.width;
        assert!(
            (card.size.width - width).abs() < 1.0,
            "карточка растянута на ширину панели {width}: {:?}",
            card.size
        );

        let actions = harness.find_by_class("system-prompt-action");
        assert_eq!(actions.len(), 4, "создать / переименовать / удалить / открыть в окне");
        for id in &actions {
            let r = right_edge(&harness, *id);
            assert!(
                r <= card_right + 0.5,
                "кнопка вылезла за карточку при ширине {width}: правый край {r}, карточка до {card_right}"
            );
            let b = harness.element_bounds(*id);
            assert!(b.size.width > 0.0 && b.size.height > 0.0, "кнопка схлопнулась: {:?}", b);
        }

        let pickers = harness.find_by_class("system-prompt-picker");
        assert_eq!(pickers.len(), 1, "один дропдаун пресетов");
        let picker = harness.element_bounds(pickers[0]);
        // Внутренняя ширина карточки: `.sampling-card { padding: 10px }`.
        let inner = width - 20.0;
        assert!(
            (picker.size.width - inner).abs() < 1.0,
            "дропдаун на всю ширину карточки при {width}: {:?}, ждали {inner}",
            picker.size
        );

        let edits = harness.find_by_class("system-prompt-edit");
        assert_eq!(edits.len(), 1, "редактор в карточке");
        assert!(right_edge(&harness, edits[0]) <= card_right + 0.5);
    }
}
