//! Настройки у каждого чата свои: инструменты, пул autotools, скилы,
//! системный промпт, сэмплинг и размышления сохраняются в файле чата и
//! возвращаются в панели при переключении, в том числе после архива.
//!
//! Зачем тест: до 19.09.2026 в файле чата жили только `syn_params`, а
//! инструменты, скилы и промпт были общими на все чаты — переключение их не
//! меняло, и настройки одного чата «протекали» в другой.
//!
//! Один `#[test]` на файл: сигналы и `HOME` общие на поток/процесс.

use syngui::prelude::*;
use syngui::signal::drain_and_run_effects;

use synthos::agent::state::ChatMsg;
use synthos::agent::storage::StoredChat;
use synthos::context::AppCtx;
use synthos::syn_chat::chat_settings::{self, ChatSettings, TurnSettings};
use synthos::syn_chat::prompt_presets::{self, PromptDialog};
use synthos::syn_chat::{autosave, registry, storage, SynChatCtx};

fn set_panels(app: &AppCtx, ctx: &SynChatCtx, s: &ChatSettings) {
    app.tools.active.set(s.tools_active.clone());
    app.tools.auto.set(s.tools_auto.clone());
    app.skills_active.set(s.skills_active.clone());
    ctx.system_prompt.set(s.system_prompt.clone());
    ctx.prompt_active.set(s.prompt_preset.clone());
}

fn on_disk(id: &str) -> ChatSettings {
    storage::load(id)
        .and_then(|c| c.settings)
        .unwrap_or_else(|| panic!("у чата {id} нет настроек в файле"))
}

#[test]
fn settings_follow_the_open_chat() {
    let home = std::env::temp_dir().join(format!("synthos-chat-settings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);
    syngui::signal::allow_signal_reads_on_this_thread();
    syngui::i18n::register_catalogs(&synthos::i18n::CATALOGS);

    let (_theme, app) = synthos::build_context();
    provide_context(app.clone());
    let ctx = SynChatCtx::new();
    provide_context(ctx.clone());

    // ── Чат, записанный до переезда настроек в файл ────────────────────
    storage::save(&StoredChat {
        id: "legacy".into(),
        title: "старый".into(),
        created_at: 1,
        updated_at: 1,
        messages: vec![ChatMsg::user("привет")],
        ..StoredChat::default()
    });
    // Общие настройки того времени — в панелях при старте. Промпт тогда
    // жил в активном пресете библиотеки, и текст в панели совпадал с ним.
    let first_preset = ctx.prompt_active.get_untracked();
    ctx.prompt_presets.update(|list| {
        for p in list.iter_mut().filter(|p| p.id == first_preset) {
            p.text = "общий промпт".into();
        }
    });
    let global = ChatSettings {
        tools_active: vec!["bash".into(), "notes".into()],
        tools_auto: vec!["pipelines".into()],
        skills_active: vec!["tone".into()],
        system_prompt: "общий промпт".into(),
        prompt_preset: first_preset,
    };
    set_panels(&app, &ctx, &global);
    registry::load_all();
    assert_eq!(ctx.active_chat_id.get_untracked().as_deref(), Some("legacy"));
    assert_eq!(on_disk("legacy"), global, "старый чат получил общие настройки на старте");
    assert_eq!(ChatSettings::capture(&ctx), global);

    // ── Новый чат начинает с настроек открытого ───────────────────────
    let b = registry::create_new();
    storage::flush(&b);
    assert_eq!(on_disk(&b), global);
    let b_settings = ChatSettings {
        tools_active: vec!["web".into()],
        tools_auto: vec![],
        skills_active: vec![],
        system_prompt: "промпт чата B".into(),
        prompt_preset: String::new(),
    };
    set_panels(&app, &ctx, &b_settings);
    ctx.params.update(|p| {
        p.enable_thinking = false;
        p.reasoning_effort = "low".into();
        p.temperature = 0.3;
    });
    drain_and_run_effects();
    autosave::flush();
    storage::flush(&b);
    assert_eq!(on_disk(&b), b_settings, "правка настроек записана в файл чата");
    let b_params = storage::load(&b).unwrap().syn_params.unwrap();
    assert!(!b_params.enable_thinking && b_params.reasoning_effort == "low");

    // ── Переключения туда и обратно ────────────────────────────────────
    registry::select("legacy");
    assert_eq!(ChatSettings::capture(&ctx), global, "вернулись настройки старого чата");
    assert!(ctx.params.get_untracked().enable_thinking, "размышления старого чата");
    assert_eq!(on_disk(&b), b_settings, "уход из чата его настроек не тронул");
    registry::select(&b);
    assert_eq!(ChatSettings::capture(&ctx), b_settings);
    assert_eq!(ctx.params.get_untracked().temperature, 0.3);
    assert_eq!(ctx.params.get_untracked().reasoning_effort, "low");

    // Переключение само по себе не правка: `updated_at` не сдвигается.
    let stamp = storage::load("legacy").unwrap().updated_at;
    registry::select("legacy");
    drain_and_run_effects();
    autosave::flush();
    storage::flush("legacy");
    assert_eq!(storage::load("legacy").unwrap().updated_at, stamp);

    // ── Архив и возврат из архива ──────────────────────────────────────
    registry::select(&b);
    registry::archive(&b);
    assert_eq!(ctx.active_chat_id.get_untracked().as_deref(), Some("legacy"));
    assert_eq!(ChatSettings::capture(&ctx), global);
    let archived = storage::load(&b).unwrap();
    assert!(archived.archived);
    assert_eq!(archived.settings.as_ref(), Some(&b_settings), "в архиве настройки на месте");
    registry::unarchive(&b);
    assert_eq!(ctx.active_chat_id.get_untracked().as_deref(), Some(b.as_str()));
    assert_eq!(ChatSettings::capture(&ctx), b_settings, "из архива вернулись настройки");
    assert!(!ctx.params.get_untracked().enable_thinking);

    // ── Ход, ушедший в фон, доигрывает со своими настройками ───────────
    ctx.generating_chat.set(Some(b.clone()));
    ctx.turn_settings.set(Some(TurnSettings::capture(&ctx)));
    registry::select("legacy");
    let turn = chat_settings::for_turn(&ctx);
    assert_eq!(turn.chat, b_settings, "autotools/subagent хода видят инструменты его чата");
    assert!(!turn.params.enable_thinking);
    ctx.generating_chat.set(None);
    assert_eq!(chat_settings::for_turn(&ctx).chat, global, "хода нет — настройки открытого");

    // ── Промпт: своя копия у чата, библиотека — заготовки ──────────────
    registry::select(&b);
    prompt_presets::create(&ctx, "Из чата B", true);
    let preset = ctx.prompt_active.get_untracked();
    assert!(!preset.is_empty());
    assert_eq!(ctx.system_prompt.get_untracked(), "промпт чата B", "копия текста не сбросила чат");
    assert!(!prompt_presets::is_modified(&ctx));

    ctx.system_prompt.set("промпт чата B, правка".into());
    assert!(prompt_presets::is_modified(&ctx));
    let lib_text = |id: &str| {
        ctx.prompt_presets
            .get_untracked()
            .into_iter()
            .find(|p| p.id == id)
            .map(|p| p.text)
    };
    assert_eq!(lib_text(&preset).as_deref(), Some("промпт чата B"), "правка чата не ушла в пресет");

    // Другой чат берёт тот же пресет — правка B до него не доходит.
    registry::select("legacy");
    prompt_presets::request_select(&ctx, &preset);
    assert_eq!(ctx.prompt_dialog.get_untracked(), None, "у старого чата текст из пресета — терять нечего");
    assert_eq!(ctx.system_prompt.get_untracked(), "промпт чата B");
    registry::select(&b);
    assert_eq!(ctx.system_prompt.get_untracked(), "промпт чата B, правка", "правка B пережила переключение");

    // Выбор пресета поверх правки — только после подтверждения.
    let first = ctx.prompt_presets.get_untracked()[0].clone();
    prompt_presets::request_select(&ctx, &first.id);
    assert!(
        matches!(ctx.prompt_dialog.get_untracked(), Some(PromptDialog::Replace { ref id, .. }) if *id == first.id),
        "правка чата пропала бы без вопроса"
    );
    assert_eq!(ctx.system_prompt.get_untracked(), "промпт чата B, правка");
    ctx.prompt_dialog.set(None);

    // «Сохранить в пресет» и «вернуть текст пресета».
    prompt_presets::save_to_preset(&ctx);
    assert_eq!(lib_text(&preset).as_deref(), Some("промпт чата B, правка"));
    assert!(!prompt_presets::is_modified(&ctx));
    ctx.system_prompt.set("ещё правка".into());
    prompt_presets::select(&ctx, &preset);
    assert_eq!(ctx.system_prompt.get_untracked(), "промпт чата B, правка", "вернули текст пресета");

    // Удаление пресета не трогает промпты чатов.
    prompt_presets::delete(&ctx, &preset);
    assert_eq!(ctx.system_prompt.get_untracked(), "промпт чата B, правка");
    assert_eq!(ctx.prompt_active.get_untracked(), "", "текст чата стал своим");
    autosave::flush();
    storage::flush(&b);
    assert_eq!(on_disk(&b).system_prompt, "промпт чата B, правка");
    registry::select("legacy");
    assert_eq!(ctx.system_prompt.get_untracked(), "промпт чата B", "копия старого чата цела");

    let _ = std::fs::remove_dir_all(&home);
}
