//! Сворачиваемые карточки боковых панелей чата
//! (`components::collapsible_card`): у каждой карточки таба «Параметры»
//! иконка и шеврон в шапке, щелчок по шапке сворачивает тело, щелчок по
//! кнопке в шапке — нет, раскрытие уходит в конфиг и возвращается из него.
//!
//! Зачем тест: шапка — `GestureDetector` поверх ряда с кнопками. Перестань
//! кнопка брать нажатие сама, «Новый промпт» заодно сворачивал бы карточку
//! «Система». А тело живёт в `AnimatedSize`: не отдай он размер при первом
//! layout, раскрытая карточка рисовалась бы пустой шапкой.
//!
//! Секции левой панели собраны тем же компонентом, но читают `AppCtx`,
//! которого в harness нет, — их здесь не рендерим.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::prelude::*;
use syngui::testing::*;

use synthos::config::{AppConfig, SynChatCardsConfig};
use synthos::pages::syn_chat::right_panel;
use synthos::syn_chat::prompt_presets::PromptDialog;
use synthos::syn_chat::{SynChatCtx, SynModelRegistry};

const WIDTH: f32 = 300.0;

/// Карточка — в столбце: корень harness растягивается на весь viewport, и
/// высота свёрнутой не отличалась бы от раскрытой.
fn render(widget: Box<dyn Widget>) -> TestHarness {
    let column = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(vec![widget]);
    let mut h = TestHarness::new(Box::new(column));
    let engine = h.apply_mss(synthos::styles::styles());
    h.rebuild();
    h.apply_styles(&engine);
    h.layout(WIDTH, 1400.0);
    h
}

fn center(h: &TestHarness, id: ElementId) -> Point {
    let b = h.element_bounds(id);
    Point::new(b.origin.x + b.size.width / 2.0, b.origin.y + b.size.height / 2.0)
}

fn card_height(h: &TestHarness) -> f32 {
    let cards = h.find_by_class("sampling-card");
    assert_eq!(cards.len(), 1, "одна карточка");
    h.element_bounds(cards[0]).size.height
}

#[test]
fn params_cards_collapse_by_header_and_persist() {
    let home = std::env::temp_dir().join(format!("synthos-collapsible-cards-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);
    syngui::i18n::register_catalogs(&synthos::i18n::CATALOGS);
    syngui::i18n::set_language(syngui::i18n::Lang::new("ru"));

    provide_context(SynChatCtx::new());
    provide_context(SynModelRegistry::new());
    let ctx = use_context::<SynChatCtx>();

    // ── Дефолты: всё раскрыто, кроме Sampling; старый конфиг без поля ──
    assert_eq!(ctx.cards.to_config(), SynChatCardsConfig::default());
    let d = SynChatCardsConfig::default();
    assert!(!d.sampling, "Sampling свёрнут по умолчанию");
    assert!(d.tools && d.autotools && d.skills && d.model && d.thinking && d.context && d.system);
    let partial: SynChatCardsConfig = serde_json::from_str(r#"{"model": false}"#).unwrap();
    assert!(!partial.model && partial.context && !partial.sampling, "недостающие флаги — дефолты");

    // ── Каждая карточка: иконка + шеврон, тело только у раскрытой ──────
    ctx.cards.sampling.set(true);
    let cards: [(&str, fn() -> Box<dyn Widget>, RwSignal<bool>); 5] = [
        ("model", || Box::new(right_panel::model_card()), ctx.cards.model),
        ("thinking", || Box::new(right_panel::thinking_card()), ctx.cards.thinking),
        ("sampling", || Box::new(right_panel::sampling_card()), ctx.cards.sampling),
        ("context", || Box::new(right_panel::context_card()), ctx.cards.context),
        ("system", || Box::new(right_panel::system_prompt_card()), ctx.cards.system),
    ];
    for (name, build, open) in cards {
        let h = render(build());
        assert_eq!(h.find_by_class("collapsible-card-icon").len(), 1, "{name}: иконка в шапке");
        assert_eq!(h.find_by_class("collapsible-card-chevron").len(), 1, "{name}: шеврон");
        let bodies = h.find_by_class("collapsible-card-body");
        assert_eq!(bodies.len(), 1, "{name}: тело раскрытой карточки");
        assert!(h.element_bounds(bodies[0]).size.height > 10.0, "{name}: тело схлопнуто");
        let open_height = card_height(&h);

        open.set(false);
        let h = render(build());
        assert!(h.find_by_class("collapsible-card-body").is_empty(), "{name}: тело свёрнутой карточки");
        assert_eq!(h.find_by_class("collapsible-card-chevron").len(), 1, "{name}: шеврон у свёрнутой");
        let closed_height = card_height(&h);
        assert!(
            closed_height + 10.0 < open_height,
            "{name}: свёрнутая не ниже раскрытой: {closed_height} против {open_height}"
        );
        open.set(true);
    }

    // ── Щелчок по шапке сворачивает ────────────────────────────────────
    let mut h = render(Box::new(right_panel::context_card()));
    let title = h.find_by_class("collapsible-card-title")[0];
    h.send_events(&click_at(center(&h, title)));
    assert!(!ctx.cards.context.get_untracked(), "щелчок по заголовку свернул «Контекст»");

    // ── Щелчок по кнопке в шапке — действие, а не сворачивание ─────────
    let mut h = render(Box::new(right_panel::system_prompt_card()));
    let actions = h.find_by_class("system-prompt-action");
    assert_eq!(actions.len(), 4, "кнопки библиотеки в шапке раскрытой карточки");
    h.send_events(&click_at(center(&h, actions[0])));
    assert!(ctx.cards.system.get_untracked(), "кнопка «Новый промпт» свернула карточку");
    assert!(
        matches!(ctx.prompt_dialog.get_untracked(), Some(PromptDialog::Create)),
        "кнопка «Новый промпт» не сработала"
    );
    ctx.prompt_dialog.set(None);

    ctx.cards.system.set(false);
    let h = render(Box::new(right_panel::system_prompt_card()));
    assert!(h.find_by_class("system-prompt-action").is_empty(), "кнопки у свёрнутой карточки");

    // ── Раскрытие — в конфиг и обратно ─────────────────────────────────
    let snap = ctx.cards.to_config();
    assert!(!snap.context && !snap.system && snap.model);
    let cfg = AppConfig {
        syn_chat_cards: snap,
        ..AppConfig::default()
    };
    let back: AppConfig = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
    assert_eq!(back.syn_chat_cards, snap);
}
