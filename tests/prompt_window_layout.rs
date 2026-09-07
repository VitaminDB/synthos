//! Плавающее окно системного промпта: редактор занимает всю площадь окна и
//! растёт вместе с ним.
//!
//! Зачем тест: `FloatingWindow` отдаёт детям высоту содержимого только через
//! `explicit_dimensions`, а `Column` раздаёт остаток лишь flex-детям. Если
//! у `.system-prompt-window-edit` пропадёт `flex-grow` или окно перестанет
//! сообщать высоту, редактор сожмётся до `rows(12)` и большой промпт снова
//! придётся крутить в узком поле — ровно то, ради чего окно и заведено.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use syngui::prelude::*;
use syngui::testing::TestHarness;

use synthos::pages::syn_chat::prompt_window;
use synthos::syn_chat::state::PROMPT_WINDOW_DEFAULT_SIZE;
use synthos::syn_chat::SynChatCtx;

/// Высота заголовка и отступ содержимого `FloatingWindow` (константы
/// syngui `TITLE_BAR_HEIGHT` / `.system-prompt-window { padding: 12px }`).
const TITLE_BAR: f32 = 36.0;
const PADDING: f32 = 12.0;

/// Как кадр приложения: реактивные ветки достраиваются при `rebuild`, новым
/// элементам нужны стили, потом раскладка.
fn settle(harness: &mut TestHarness, engine: &syngui::mss::StyleEngine) {
    harness.rebuild();
    harness.apply_styles(engine);
    harness.layout(1400.0, 900.0);
}

fn editor_bounds(harness: &TestHarness) -> Rect {
    let ids = harness.find_by_class("system-prompt-window-edit");
    assert_eq!(ids.len(), 1, "в окне ровно один редактор");
    harness.element_bounds(ids[0])
}

#[test]
fn editor_fills_window_and_grows_with_it() {
    // Конструктор контекста читает `~/.config/synthos` — уводим HOME во
    // временный каталог, чтобы тест не трогал настройки пользователя.
    let home = std::env::temp_dir().join(format!("synthos-prompt-window-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);

    provide_context(SynChatCtx::new());
    let ctx = use_context::<SynChatCtx>();
    ctx.system_prompt.set("строка 1\nстрока 2".to_string());
    ctx.prompt_window_open.set(true);

    let mut harness = TestHarness::new(Box::new(
        Stack::new().fit(StackFit::Expand).child(prompt_window::window()),
    ));
    let engine = harness.apply_mss(synthos::styles::styles());
    settle(&mut harness, &engine);

    let content_w = PROMPT_WINDOW_DEFAULT_SIZE.width - 2.0 * PADDING;
    let content_h = PROMPT_WINDOW_DEFAULT_SIZE.height - TITLE_BAR - 2.0 * PADDING;
    let b = editor_bounds(&harness);
    assert!(
        (b.size.width - content_w).abs() < 1.0,
        "редактор на всю ширину содержимого: {:?}, ждали {content_w}",
        b.size
    );
    // Под редактором строка статистики (~16px) и зазор 8px — остальное его.
    assert!(
        b.size.height > content_h - 40.0 && b.size.height < content_h,
        "редактор должен занять остаток высоты окна: {:?}, содержимое {content_h}",
        b.size
    );

    // Пользователь растянул окно — редактор растёт вместе с ним.
    ctx.prompt_window_size.set(Size::new(1100.0, 820.0));
    settle(&mut harness, &engine);
    let grown = editor_bounds(&harness);
    assert!(
        grown.size.height > b.size.height + 200.0 && grown.size.width > b.size.width + 300.0,
        "после ресайза редактор больше: было {:?}, стало {:?}",
        b.size,
        grown.size
    );
}
