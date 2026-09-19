//! Headless-прогон agent-loop'а Syn-чата на реальной модели — без окна, но
//! теми же кубиками, что в GUI: `send_message` → `run_agent_loop` →
//! парсер вызовов → инструменты → лента. Main-thread колбэки воркера
//! выкачиваются `drain_main_thread_callbacks`, как в `llm_smoke`.
//!
//! Зачем: сломанные вызовы инструментов (см.
//! `docs/chat_tool_call_robustness_2026.md`) из юнит-тестов не видны —
//! нужны живая модель, живой сэмплинг и несколько сообщений подряд.
//!
//! Запуск (из каталога проекта — он же рабочий каталог `bash`):
//! `agent_smoke <bundle.syn> <prompt> [prompt2 …]`; промпт вида `@путь`
//! читается из файла — аргумент командной строки ограничен 128 КБ, а
//! промпт на 65k токенов в него не помещается.
//!
//! Переменные окружения:
//! - `SYN_SMOKE_TURNS` — бюджет ходов агента на сообщение (по умолчанию 24);
//! - `SYN_SMOKE_SUB_TURNS` — бюджет ходов субагента (по умолчанию столько же:
//!   у него свой лимит из конфига, и без этого один вызов субагента тянет
//!   прогон на десятки минут);
//! - `SYN_SMOKE_TOOLS` — активные инструменты через запятую (пустая строка —
//!   без инструментов); без переменной — `tools_active` из конфига, как в GUI;
//! - `SYN_SMOKE_AUTO` — пул `autotools` через запятую (пустая строка — пул
//!   пуст); без переменной — `tools_auto` из конфига;
//! - `SYN_SMOKE_PARAMS` — JSON с полями `SamplingParams` поверх
//!   `syn_chat_defaults` из конфига (например
//!   `{"temperature":0.0,"enable_thinking":false}`); без переменной — ровно
//!   параметры, с которыми модель запускает страница чата;
//! - `SYN_SMOKE_TIMEOUT_S` — потолок на одно сообщение (по умолчанию 1800);
//! - `SYN_SMOKE_OUT` — куда сохранить ленту (JSON, как `syn_chats/*.json`);
//! - `SYN_SMOKE_SWITCH_AFTER_S` — через сколько секунд после старта первого
//!   сообщения «уйти» в другой чат: проверка того, что ход доигрывает в фоне
//!   и пишет в ленту своего чата, а не в открытую.
//!
//! Подтверждения инструментов выключены (`tools.allow_all`): раннер
//! проверяет модель, а не диалоги.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use syngui::context_provider::provide_context;
use synthos::agent::state::{ChatMsg, ChatMsgKind, ChatMsgRole};
use synthos::syn_chat::params::SamplingParams;
use synthos::syn_chat::{session, SynChatCtx, SynModelRegistry};

fn drain() {
    syngui::async_runtime::drain_main_thread_callbacks();
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn head(s: &str, n: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c == '\n' { '⏎' } else { c })
        .take(n)
        .collect();
    if s.chars().count() > n {
        format!("{flat}…")
    } else {
        flat
    }
}

fn describe(i: usize, m: &ChatMsg) -> String {
    let role = match m.role {
        ChatMsgRole::User => "user",
        ChatMsgRole::Assistant => "assistant",
        ChatMsgRole::System => "system",
    };
    match &m.kind {
        ChatMsgKind::Text => format!(
            "#{i:3} {} {role:9} text     {}{}",
            m.time,
            head(&m.body, 220),
            if m.thinking.is_empty() {
                String::new()
            } else {
                format!(" [thinking {} chars]", m.thinking.chars().count())
            }
        ),
        ChatMsgKind::ToolCall { tool_name } => format!(
            "#{i:3} {} {role:9} CALL     {tool_name}: {}",
            m.time,
            head(&m.body, 220)
        ),
        ChatMsgKind::ToolResult {
            tool_name, error, ..
        } => format!(
            "#{i:3} {} {role:9} result   {tool_name}{}: {}",
            m.time,
            if *error { " (error)" } else { "" },
            head(&m.body, 160)
        ),
        ChatMsgKind::CompactionMarker {
            compacted_count, ..
        } => format!("#{i:3} {} {role:9} compact  {compacted_count} сообщений свёрнуто", m.time),
    }
}

/// Итог одного сообщения: что стоит проверить глазами.
#[derive(Default)]
struct Verdict {
    calls: usize,
    empty_calls: usize,
    bad_args: usize,
    guard: usize,
    xml_or_dup_name: usize,
    answered: bool,
}

fn verdict(msgs: &[ChatMsg], from: usize) -> Verdict {
    let mut v = Verdict::default();
    for m in &msgs[from..] {
        match &m.kind {
            ChatMsgKind::ToolCall { tool_name } => {
                v.calls += 1;
                if m.body.trim() == "{}" {
                    v.empty_calls += 1;
                }
                if tool_name.contains('"') || tool_name.contains('<') {
                    v.xml_or_dup_name += 1;
                }
            }
            ChatMsgKind::ToolResult { .. } => {
                if m.body.starts_with("Missing required field")
                    || m.body.starts_with("Invalid arguments")
                    || m.body.starts_with("Unknown tool")
                {
                    v.bad_args += 1;
                }
                if m.body.contains("только что выполнялся")
                    || m.body.contains("Остановлено:")
                    || m.body.contains("чередуется")
                {
                    v.guard += 1;
                }
            }
            ChatMsgKind::Text if m.role == ChatMsgRole::Assistant => {
                if !m.body.trim().is_empty() {
                    v.answered = true;
                }
            }
            _ => {}
        }
    }
    v
}

fn run() -> std::result::Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        return Err(format!(
            "использование: {} <bundle.syn> <prompt> [prompt2 …]",
            args[0]
        ));
    }
    let bundle = std::path::PathBuf::from(&args[1]);
    if !bundle.exists() {
        return Err(format!("нет бандла: {}", bundle.display()));
    }
    let mut prompts: Vec<String> = Vec::with_capacity(args.len() - 2);
    for a in &args[2..] {
        prompts.push(match a.strip_prefix('@') {
            Some(path) => std::fs::read_to_string(path)
                .map_err(|e| format!("промпт из файла {path}: {e}"))?,
            None => a.clone(),
        });
    }
    let turns: u32 = env_or("SYN_SMOKE_TURNS", 24);
    let sub_turns: u32 = env_or("SYN_SMOKE_SUB_TURNS", turns);
    let timeout = Duration::from_secs(env_or("SYN_SMOKE_TIMEOUT_S", 1800));
    // Конфиг — тот же, что читает GUI (`$HOME/.config/synthos/config.json`):
    // прогон обязан идти на параметрах и инструментах приложения, иначе
    // «на тестах работало» ничего не говорит о чате.
    let cfg = synthos::config::AppConfig::load();
    let key_list = |list: String| -> Vec<String> {
        list.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };
    let tools: Vec<String> = match std::env::var("SYN_SMOKE_TOOLS") {
        Ok(list) => key_list(list),
        Err(_) => cfg.tools_active.clone(),
    };
    let auto: Vec<String> = match std::env::var("SYN_SMOKE_AUTO") {
        Ok(list) => key_list(list),
        Err(_) => cfg.tools_auto.clone(),
    };
    let switch_after: u64 = env_or("SYN_SMOKE_SWITCH_AFTER_S", 0);
    let params: SamplingParams = match std::env::var("SYN_SMOKE_PARAMS") {
        Ok(json) => {
            let mut base = serde_json::to_value(&cfg.syn_chat_defaults).map_err(|e| e.to_string())?;
            let over: serde_json::Value =
                serde_json::from_str(&json).map_err(|e| format!("SYN_SMOKE_PARAMS: {e}"))?;
            match (base.as_object_mut(), over.as_object()) {
                (Some(b), Some(o)) => b.extend(o.iter().map(|(k, v)| (k.clone(), v.clone()))),
                _ => return Err("SYN_SMOKE_PARAMS: ожидался JSON-объект".into()),
            }
            serde_json::from_value(base).map_err(|e| format!("SYN_SMOKE_PARAMS: {e}"))?
        }
        Err(_) => cfg.syn_chat_defaults.clone(),
    };
    eprintln!(
        "agent_smoke: bundle={} turns={turns} sub_turns={sub_turns} tools={tools:?} auto={auto:?} params={}",
        bundle.display(),
        serde_json::to_string(&params).unwrap_or_default()
    );

    // Контексты — как в `run_desktop`, но без окна и без страниц.
    let (_theme, app_ctx) = synthos::build_context();
    // Тексты ошибок и подписи проходят через `tr!` — без каталога в ленте
    // остаются ключи вида `chat.session.error.stopped_suffix`.
    synthos::i18n::install(app_ctx.general);
    app_ctx.tools.allow_all.set(true);
    app_ctx.tools.active.set(tools);
    app_ctx.tools.auto.set(auto);
    app_ctx.general.agent_max_turns.set(turns);
    app_ctx.general.subagent_max_turns.set(sub_turns);
    provide_context(app_ctx.clone());
    let chat = SynChatCtx::new();
    let chat_id = format!("smoke-{}", synthos::agent::time::unix_secs());
    chat.active_chat_id.set(Some(chat_id.clone()));
    chat.params.set(params.clone());
    // Файл чата нужен на диске: ход, ушедший в фон, пишет ленту прямо в него
    // (`session::ledger_update`), а без файла правки было бы некуда класть.
    synthos::syn_chat::storage::save(&synthos::agent::storage::StoredChat {
        id: chat_id.clone(),
        title: "agent_smoke".into(),
        created_at: synthos::agent::time::unix_secs(),
        updated_at: synthos::agent::time::unix_secs(),
        model_name: bundle.file_stem().and_then(|s| s.to_str()).map(String::from),
        messages: Vec::new(),
        syn_params: Some(params),
        // Инструменты хода берутся из сигналов выше; файлу они не нужны.
        settings: None,
        archived: false,
    });
    provide_context(chat.clone());
    let registry = SynModelRegistry::new();
    provide_context(registry);
    // Контексты страниц: инструмент `notes` работает с деревом заметок, а
    // `pipelines` — с рабочим столом графов. Без них вызов инструмента valит
    // весь процесс на `use_context`.
    provide_context(synthos::pages::notes::NotesCtx::new_or_restore(&cfg));
    provide_context(synthos::pages::node_editor::tabs::EditorWorkspace::new_or_restore());

    // Модель — той же политикой, что выбрала бы страница чата.
    let policy = synthos::config::resolve_model_profile(
        &app_ctx.model_profiles.get_untracked(),
        &bundle,
    )
    .policy;
    eprintln!("agent_smoke: профиль {}", policy.preset_name);
    let t_load = Instant::now();
    registry.load(bundle.clone(), policy);
    loop {
        drain();
        if let Some(e) = registry.error.get_untracked() {
            return Err(format!("загрузка модели: {e}"));
        }
        if registry.current.get_untracked().is_some() && !registry.loading.get_untracked() {
            break;
        }
        if t_load.elapsed() > Duration::from_secs(1200) {
            return Err("модель не загрузилась за 20 минут".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    eprintln!("agent_smoke: модель загружена за {:.1}s", t_load.elapsed().as_secs_f32());

    let mut overall_ok = true;
    for (pi, prompt) in prompts.iter().enumerate() {
        let from = chat.messages.with_untracked(Vec::len);
        println!("\n════════ сообщение {} / {}: {}", pi + 1, prompts.len(), head(prompt, 200));
        session::send_message(prompt.clone());
        drain();
        let t0 = Instant::now();
        let mut switched = false;
        // Лента не append-only: пустой плейсхолдер ассистента заменяется
        // пузырём вызова, текст дописывается стримом. Поэтому помним, что
        // печатали, и перепечатываем изменившиеся строки.
        let mut shown: Vec<String> = Vec::new();
        let mut last_tick = Instant::now();
        let mut aborted = false;
        let sync_print = |msgs: &[ChatMsg], shown: &mut Vec<String>| {
            for (i, m) in msgs.iter().enumerate().skip(from) {
                let line = describe(i, m);
                let k = i - from;
                if k < shown.len() {
                    if shown[k] != line {
                        println!("{line}   ← обновлено");
                        shown[k] = line;
                    }
                } else {
                    println!("{line}");
                    shown.push(line);
                }
            }
        };
        loop {
            drain();
            let msgs = chat.messages.get_untracked();
            sync_print(&msgs, &mut shown);
            // Ждём именно владельца хода: `pending` относится к открытому
            // чату и гаснет, как только мы «ушли» в другой.
            if chat.generating_chat.get_untracked().is_none() {
                break;
            }
            if last_tick.elapsed() > Duration::from_secs(15) {
                last_tick = Instant::now();
                eprintln!(
                    "  … {:.0}s: сообщений {}, стрим body={} think={} tool={}",
                    t0.elapsed().as_secs_f32(),
                    msgs.len(),
                    chat.streaming_body.get_untracked().chars().count(),
                    chat.streaming_thinking.get_untracked().chars().count(),
                    chat.streaming_tool.get_untracked().chars().count()
                );
            }
            // Имитация «ушёл в другой чат»: активным становится посторонний
            // id, лента опустошается. Ход обязан доиграть и записать всё в
            // файл своего чата.
            if switch_after > 0 && !switched && pi == 0 && t0.elapsed().as_secs() >= switch_after {
                switched = true;
                let own = chat.active_chat_id.get_untracked().unwrap_or_default();
                println!("──── ушли из чата {own} в другой (проверка фоновой генерации)");
                chat.active_chat_id.set(Some(format!("{own}-other")));
                chat.messages.set(Vec::new());
                chat.pending.set(false);
            }
            if !aborted && t0.elapsed() > timeout {
                eprintln!("agent_smoke: таймаут {timeout:?} — прерываем ход");
                chat.abort.fetch_add(1, Ordering::Relaxed);
                aborted = true;
                overall_ok = false;
            }
            if aborted && t0.elapsed() > timeout + Duration::from_secs(120) {
                return Err("ход не остановился после прерывания".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        drain();
        if switched {
            // Возвращаемся: лента чата читается с диска — ровно так же, как
            // это делает `registry::select` в приложении.
            let own = chat
                .active_chat_id
                .get_untracked()
                .unwrap_or_default()
                .trim_end_matches("-other")
                .to_string();
            let restored = synthos::syn_chat::storage::load(&own)
                .map(|c| c.messages)
                .unwrap_or_default();
            println!(
                "──── вернулись в чат {own}: в файле {} сообщений",
                restored.len()
            );
            chat.active_chat_id.set(Some(own));
            chat.messages.set(restored);
            shown.clear();
        }
        let msgs = chat.messages.get_untracked();
        sync_print(&msgs, &mut shown);
        let v = verdict(&msgs, from);
        let err = chat.error.get_untracked();
        println!(
            "──── итог сообщения {}: {:.0}s, вызовов {}, пустых {}, битых аргументов {}, guard {}, \
             имя с мусором {}, текстовый ответ: {}, turn_cap: {}, ошибка: {}",
            pi + 1,
            t0.elapsed().as_secs_f32(),
            v.calls,
            v.empty_calls,
            v.bad_args,
            v.guard,
            v.xml_or_dup_name,
            if v.answered { "да" } else { "НЕТ" },
            chat.turn_cap_reached.get_untracked(),
            err.clone().unwrap_or_else(|| "нет".into())
        );
        println!(
            "     статистика хода: prompt={} ток, из кэша={} ток, ходов={}, сгенерировано={} ток, \
             prefill={} мс, decode={:.1} ток/с, VRAM свободно={} MB",
            chat.last_prompt_tokens.get_untracked(),
            chat.last_reused_tokens.get_untracked(),
            chat.last_turns.get_untracked(),
            chat.last_gen_tokens.get_untracked(),
            chat.last_prefill_ms.get_untracked(),
            chat.last_decode_tps.get_untracked(),
            chat.last_vram_free_mb.get_untracked()
        );
        if !v.answered || v.empty_calls > 0 || v.bad_args > 0 || v.xml_or_dup_name > 0 || err.is_some() {
            overall_ok = false;
        }
    }

    if let Ok(out) = std::env::var("SYN_SMOKE_OUT") {
        let msgs = chat.messages.get_untracked();
        let json = serde_json::to_string_pretty(&msgs).map_err(|e| e.to_string())?;
        std::fs::write(&out, json).map_err(|e| format!("{out}: {e}"))?;
        eprintln!("agent_smoke: лента сохранена в {out}");
    }

    println!(
        "\nSMOKE_RESULT bundle={} prompts={} ok={}",
        bundle.file_name().and_then(|s| s.to_str()).unwrap_or("?"),
        prompts.len(),
        overall_ok
    );
    registry.unload();
    drain();
    if overall_ok {
        Ok(())
    } else {
        Err("есть замечания, см. итоги сообщений выше".into())
    }
}

fn main() {
    syngui::signal::init_main_thread();
    synthos::logging::init();
    if let Err(e) = run() {
        eprintln!("agent_smoke FAILED: {e}");
        std::process::exit(1);
    }
}
