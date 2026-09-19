//! Автосейв ленты открытого Syn-чата.
//!
//! Раньше файл чата писал сам эффект, то есть кадр: эффекты выполняются в
//! `render` перед layout, и каждое изменение `messages` стоило клона и хэша
//! всей истории, чтения файла с диска ради `created_at`, pretty-JSON и
//! `fs::write`, а `refresh_active_preview` правил `chats` и будил тот же
//! эффект второй раз. Теперь эффекты ([`install`]) только отмечают правку.
//! Снимок снимается один раз после паузы [`DEBOUNCE`] (при непрерывных
//! правках — не реже [`MAX_WAIT`]), метаданные берутся из `ChatMeta`, а
//! сериализацию и запись по порядку делает фоновый писатель
//! [`storage`](super::storage).
//!
//! Не дожидаясь паузы, открытый чат пишется ([`flush`]) перед сменой чата,
//! созданием нового и сборкой мусора CAS (`registry`), при открытии поиска
//! и при выходе ([`flush_for_exit`]).

use std::cell::RefCell;
use std::time::{Duration, Instant};

use syngui::prelude::*;

use super::chat_settings::ChatSettings;
use super::registry;
use super::state::SynChatCtx;
use super::storage;

/// Пауза после последней правки, после которой лента уходит на диск.
pub const DEBOUNCE: Duration = Duration::from_millis(500);
/// Потолок ожидания при непрерывных правках (длинный агентный ход):
/// аварийный выход не должен терять больше нескольких секунд ленты.
pub const MAX_WAIT: Duration = Duration::from_secs(3);

/// Таймер — через tokio и `run_on_main_thread`. В юнит-тестах не взводится:
/// очередь main-thread-колбэков общая на процесс, а время тесты двигают
/// сами ([`save_if_due`]).
const ARM_TIMER: bool = !cfg!(test);

#[derive(Default)]
struct Pending {
    /// Когда писать: пауза после последней правки.
    deadline: Option<Instant>,
    /// Первая несохранённая правка — от неё отсчитывается [`MAX_WAIT`].
    since: Option<Instant>,
    /// Таймер в пути (больше одного не нужно).
    armed: bool,
    /// `(id, название)` открытого чата, какими их видел эффект списка.
    seen_title: Option<(String, String)>,
    /// Сколько снимков отдано писателю.
    saves: u64,
}

thread_local! {
    // Автосейв живёт на главном потоке, как и сигналы, которые он читает.
    static PENDING: RefCell<Pending> = RefCell::new(Pending::default());
}

/// Эффекты автосейва. Зовётся один раз из `lib.rs::install_syn_chat_autosave`.
pub fn install(ctx: &SynChatCtx) {
    let c = ctx.clone();
    create_effect(move || {
        // Только подписка: значения читает снимок по таймеру; `with` не
        // клонирует ленту.
        let active = c.active_chat_id.get();
        c.messages.with(|_| ());
        let _ = c.params.get();
        ChatSettings::track(&c);
        if active.is_none() || c.loading.get_untracked() {
            return;
        }
        mark_dirty(Instant::now());
    });

    // Список чатов правит и сам автосейв — превью, `updated_at` и порядок
    // (`registry::refresh_active_preview`); это не повод писать файл снова.
    // Записи требует только новое название открытого чата.
    let c = ctx.clone();
    create_effect(move || {
        let active = c.active_chat_id.get();
        let chats = c.chats.get();
        let Some(id) = active else { return };
        let Some(title) = chats.into_iter().find(|m| m.id == id).map(|m| m.title) else {
            return;
        };
        let renamed = PENDING.with(|p| {
            let mut p = p.borrow_mut();
            let renamed =
                matches!(&p.seen_title, Some((seen_id, seen)) if *seen_id == id && *seen != title);
            p.seen_title = Some((id, title));
            renamed
        });
        if renamed && !c.loading.get_untracked() {
            mark_dirty(Instant::now());
        }
    });
}

/// Отметить правку в момент `now`: запись откладывается на [`DEBOUNCE`].
pub(crate) fn mark_dirty(now: Instant) {
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        p.deadline = Some(now + DEBOUNCE);
        p.since.get_or_insert(now);
    });
    arm(DEBOUNCE);
}

/// Есть отложенная запись.
pub fn is_pending() -> bool {
    PENDING.with(|p| p.borrow().deadline.is_some())
}

/// Взвести таймер, если он ещё не в пути. Правки после взвода сдвигают
/// срок, а не заводят новых таймеров: тик сверяется со сроком сам и при
/// нужде взводится заново на остаток.
fn arm(delay: Duration) {
    let fresh = PENDING.with(|p| !std::mem::replace(&mut p.borrow_mut().armed, true));
    if !fresh || !ARM_TIMER {
        return;
    }
    spawn(async move {
        tokio::time::sleep(delay).await;
        run_on_main_thread(tick);
    });
}

fn tick() {
    PENDING.with(|p| p.borrow_mut().armed = false);
    save_if_due(Instant::now());
}

/// Записать, если к моменту `now` срок подошёл; иначе ждать остаток.
pub(crate) fn save_if_due(now: Instant) {
    let due = PENDING.with(|p| {
        let p = p.borrow();
        let deadline = p.deadline?;
        let hard = p.since.map_or(deadline, |s| s + MAX_WAIT);
        Some(deadline.min(hard))
    });
    let Some(at) = due else { return };
    if now < at {
        arm(at - now);
        return;
    }
    flush();
}

/// Записать открытый чат сейчас, не дожидаясь паузы. Нужна ли запись,
/// решает отпечаток, а не отметка о правке: правка, сделанная в этом же
/// тике, до эффекта ещё не дошла (эффекты выполняются в кадре).
pub fn flush() {
    cancel();
    if let Some(ctx) = try_use_context::<SynChatCtx>() {
        save_now(&ctx);
    }
}

/// Забыть отложенную запись: открытый чат записан иначе (архив).
pub fn cancel() {
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        p.deadline = None;
        p.since = None;
    });
}

/// Выход из приложения: записать открытый чат и дождаться, пока очередь
/// писателя опустеет.
pub fn flush_for_exit() {
    flush();
    storage::flush_all();
}

/// Снимок открытого чата — писателю. `false` — писать нечего (чата нет
/// или отпечаток не изменился).
fn save_now(ctx: &SynChatCtx) -> bool {
    let Some(active_id) = ctx.active_chat_id.get_untracked() else {
        return false;
    };
    let Some(meta) = ctx
        .chats
        .get_untracked()
        .into_iter()
        .find(|m| m.id == active_id)
    else {
        return false;
    };
    let params = ctx.params.get_untracked();
    let settings = ChatSettings::capture(ctx);
    // Тот же расчёт, что `registry::select_internal` кладёт в
    // `last_saved_fp` при выборе чата — иначе выбор выглядит как правка и
    // двигает чат наверх списка. Отпечаток — по ссылке: ленту клонируем,
    // только когда есть что писать.
    let fp = ctx
        .messages
        .with_untracked(|m| registry::state_fingerprint(&meta.title, m, &params, &settings));
    if ctx.last_saved_fp.get_untracked() == fp {
        return false;
    }
    let messages = ctx.messages.get_untracked();
    let model_name = registry::current_model_name().or_else(|| meta.model_name.clone());
    registry::refresh_active_preview(&messages, model_name.clone());
    storage::save_async(registry::stored_from_meta(
        &meta, messages, params, settings, model_name,
    ));
    ctx.last_saved_fp.set(fp);
    PENDING.with(|p| p.borrow_mut().saves += 1);
    true
}

#[cfg(test)]
fn saves() -> u64 {
    PENDING.with(|p| p.borrow().saves)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::{ChatMeta, ChatMsg};
    use crate::syn_chat::params::SamplingParams;
    use crate::syn_chat::storage::StoredChat;
    use syngui::signal::drain_and_run_effects;

    struct Env {
        dir: tempfile::TempDir,
        _files: storage::TestDir,
        ctx: SynChatCtx,
    }

    /// Контекст чата с установленным автосейвом; файлы чатов — во
    /// временном каталоге этого потока.
    fn env() -> Env {
        syngui::signal::allow_signal_reads_on_this_thread();
        let dir = tempfile::tempdir().expect("временный каталог");
        let files = storage::test_dir(dir.path());
        PENDING.with(|p| *p.borrow_mut() = Pending::default());
        let ctx = SynChatCtx::new();
        provide_context(ctx.clone());
        install(&ctx);
        drain_and_run_effects();
        Env {
            dir,
            _files: files,
            ctx,
        }
    }

    fn stored(id: &str, created_at: u64, bodies: &[&str]) -> StoredChat {
        StoredChat {
            id: id.into(),
            title: format!("чат {id}"),
            created_at,
            updated_at: created_at,
            model_name: None,
            messages: bodies.iter().map(|b| ChatMsg::user(*b)).collect(),
            syn_params: Some(SamplingParams::default()),
            settings: Some(ChatSettings::default()),
            archived: false,
        }
    }

    /// Чаты на диске, открыт первый — как на старте приложения.
    fn open(env: &Env, chats: &[StoredChat]) {
        for c in chats {
            storage::save(c);
        }
        env.ctx.chats.set(storage::list_meta());
        registry::select(&chats[0].id);
        drain_and_run_effects();
    }

    fn bodies(chat: &StoredChat) -> Vec<String> {
        chat.messages.iter().map(|m| m.body.clone()).collect()
    }

    fn edit(env: &Env, body: &str) {
        env.ctx.messages.update(|m| m.push(ChatMsg::user(body)));
        drain_and_run_effects();
    }

    #[test]
    fn debounce_collapses_a_series_of_edits_into_one_write() {
        let env = env();
        open(&env, &[stored("a", 7, &["привет"])]);
        save_if_due(Instant::now() + MAX_WAIT);
        assert_eq!(saves(), 0, "открытие чата — не правка");

        for i in 0..5 {
            edit(&env, &format!("правка {i}"));
        }
        assert!(is_pending(), "эффект отметил правки");
        save_if_due(Instant::now());
        assert_eq!(saves(), 0, "до паузы ничего не пишется");
        assert_eq!(storage::load("a").unwrap().messages.len(), 1);

        save_if_due(Instant::now() + DEBOUNCE);
        assert_eq!(saves(), 1, "серия правок — одна запись");
        let on_disk = storage::load("a").unwrap();
        assert_eq!(on_disk.messages.len(), 6);
        assert_eq!(on_disk.created_at, 7);

        save_if_due(Instant::now() + MAX_WAIT);
        assert_eq!(saves(), 1, "без новых правок второй записи нет");
    }

    #[test]
    fn continuous_edits_are_written_no_later_than_max_wait() {
        let env = env();
        open(&env, &[stored("w", 1, &["старт"])]);
        let t0 = Instant::now();
        edit(&env, "первая");
        // Правки каждые 250 мс: пауза так и не наступает.
        let step = Duration::from_millis(250);
        let mut t = t0;
        while t + step < t0 + MAX_WAIT - step {
            t += step;
            mark_dirty(t);
            save_if_due(t);
            assert_eq!(saves(), 0, "пауза ещё не наступила: {:?}", t - t0);
        }
        save_if_due(t0 + MAX_WAIT + Duration::from_millis(200));
        assert_eq!(saves(), 1, "потолок ожидания пишет и без паузы");
        assert_eq!(bodies(&storage::load("w").unwrap()), ["старт", "первая"]);
    }

    #[test]
    fn switching_chats_writes_the_pending_edit() {
        let env = env();
        open(&env, &[stored("a", 1, &["a1"]), stored("b", 2, &["b1"])]);
        env.ctx.messages.update(|m| m.push(ChatMsg::user("a2")));
        // Эффект ещё не дошёл: переключение в том же тике, что и правка.
        registry::select("b");
        assert_eq!(bodies(&storage::load("a").unwrap()), ["a1", "a2"]);
        let shown: Vec<String> = env
            .ctx
            .messages
            .get_untracked()
            .into_iter()
            .map(|m| m.body)
            .collect();
        assert_eq!(shown, ["b1"]);

        // Клик по уже открытому чату перечитывает его с диска — правка
        // не должна потеряться и тут.
        edit(&env, "b2");
        registry::select("b");
        assert_eq!(bodies(&storage::load("b").unwrap()), ["b1", "b2"]);
        assert_eq!(env.ctx.messages.get_untracked().len(), 2);
    }

    #[test]
    fn snapshot_takes_metadata_from_chat_meta_not_disk() {
        let env = env();
        // Файла нет вовсе: раньше `created_at` становился «сейчас», а имя
        // модели терялось, — оба брались из файла на диске.
        env.ctx.chats.set(vec![ChatMeta {
            id: "m".into(),
            title: "мета".into(),
            preview: String::new(),
            created_at: 42,
            updated_at: 42,
            model_name: Some("qwen3.8-27b".into()),
            archived: false,
        }]);
        env.ctx.active_chat_id.set(Some("m".into()));
        env.ctx.messages.set(vec![ChatMsg::user("x")]);
        drain_and_run_effects();
        flush();
        let on_disk = storage::load("m").expect("чат записан");
        assert_eq!(on_disk.created_at, 42);
        assert_eq!(on_disk.model_name.as_deref(), Some("qwen3.8-27b"));
        assert_eq!(on_disk.title, "мета");
        assert_eq!(bodies(&on_disk), ["x"]);
    }

    #[test]
    fn own_preview_refresh_does_not_wake_autosave_but_rename_does() {
        let env = env();
        open(&env, &[stored("r", 1, &["один"])]);
        edit(&env, "два");
        save_if_due(Instant::now() + DEBOUNCE);
        assert_eq!(saves(), 1);
        assert_eq!(env.ctx.chats.get_untracked()[0].preview, "два");
        // Запись обновила превью и порядок списка — эффекты видят `chats`.
        drain_and_run_effects();
        assert!(!is_pending(), "своя правка списка — не повод писать снова");

        registry::rename_active("новое имя".into());
        drain_and_run_effects();
        assert!(is_pending(), "переименование — правка");
        save_if_due(Instant::now() + DEBOUNCE);
        assert_eq!(saves(), 2);
        assert_eq!(storage::load("r").unwrap().title, "новое имя");
    }

    #[test]
    fn exit_flush_writes_the_pending_edit() {
        let env = env();
        open(&env, &[stored("q", 1, &["до"])]);
        edit(&env, "перед выходом");
        assert!(is_pending());
        flush_for_exit();
        // Файл читается мимо `storage::load` (тот сам ждёт очередь):
        // выход обязан дождаться писателя.
        let text = std::fs::read_to_string(env.dir.path().join("q.json")).expect("файл чата");
        assert!(text.contains("перед выходом"), "{text}");
        assert!(!is_pending());
    }
}
