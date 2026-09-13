//! Системный промпт агента Syn-чата.
//!
//! До этого модуля Syn-чат уходил в генерацию вообще без system-сообщения:
//! `AppConfig::syn_chat_system_prompt` по умолчанию пуст, а `build_history`
//! пустую строку не добавляет. Модель с `bash` в руках работала без роли, без
//! окружения и — главное — без критерия остановки. В сохранённых чатах это
//! видно буквально: agent-loop уходил в сотню одинаковых вызовов подряд.
//!
//! Пользовательский промпт из настроек базовые правила не заменяет, а
//! дописывается к ним: правила работы с инструментами — часть контракта
//! agent-loop'а (guard повторов, лимит ходов), терять их из-за того, что
//! пользователь вписал в поле «отвечай в рифму», нельзя.
//!
//! Важно для префикс-KV: промпт должен быть стабилен между сообщениями
//! одного чата — он лежит в самой голове контекста, и любое изменение
//! обнуляет переиспользование кэша. Поэтому здесь есть дата, но нет времени.

use crate::agent::time::format_date_today;

/// Данные окружения для промпта. Собираются на main-потоке (нужен доступ к
/// сигналам активных инструментов), сам рендер — чистая функция.
pub struct PromptEnv {
    /// Дата «YYYY-MM-DD». Время намеренно не включаем (см. модульный док).
    pub date: String,
    /// ОС/платформа — модель должна понимать, какие команды доступны.
    pub os: String,
    /// Рабочий каталог процесса: `bash -lc` стартует именно в нём.
    pub cwd: String,
    /// Лейблы активных инструментов в порядке каталога.
    pub tools: Vec<String>,
    /// Лейблы инструментов из пула `autotools`: схемы модели не объявлены,
    /// она подгружает их по запросу. Пустой — `autotools` не упоминается.
    pub pool: Vec<String>,
    /// Лимит ходов agent-loop'а на одно сообщение («Глубина основного агента»).
    pub max_turns: usize,
    /// Пользовательский промпт из настроек. Пустой — просто не добавляется.
    pub user_prompt: String,
    /// Язык интерфейса — на нём же модель должна отвечать. Родное название
    /// («Русский», «Deutsch»): английское «reply in the user's language»
    /// модели трактуют по языку промпта, а он у нас английский, и
    /// `qwen3.8-flash-next` устойчиво отвечал по-английски на русские
    /// вопросы. Название языка прямым текстом снимает двусмысленность.
    pub language: String,
}

impl PromptEnv {
    /// Снимок окружения процесса. Инструменты и лимит ходов передаёт
    /// вызывающий — они живут в сигналах `AppCtx`.
    pub fn snapshot(
        tools: Vec<String>,
        pool: Vec<String>,
        max_turns: usize,
        user_prompt: String,
    ) -> Self {
        Self {
            date: format_date_today(),
            os: format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH),
            cwd: std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            tools,
            pool,
            max_turns,
            user_prompt,
            language: ui_language_name(),
        }
    }
}

/// Родное название текущего языка интерфейса («Русский», «English»).
/// Неизвестный тег отдаём как есть — модель поймёт и BCP-47.
fn ui_language_name() -> String {
    let lang = syngui::i18n::language();
    syngui::i18n::languages()
        .into_iter()
        .find(|l| l.tag == lang)
        .map(|l| l.name)
        .unwrap_or_else(|| lang.tag().to_string())
}

/// Собрать итоговый system-промпт.
pub fn build(env: &PromptEnv) -> String {
    let mut s = String::with_capacity(1024);
    s.push_str(
        "You are Syn, a local AI agent in the Synthos app. You run on the \
         user's machine.\n\n",
    );

    s.push_str(&format!(
        "Language: answer in {lang} — this includes your reasoning, your \
         explanations and the one-line announcements before tool calls. \
         These instructions are in English; your replies are not. Switch \
         only if the user writes to you in another language: then match the \
         language of their message.\n\n",
        lang = env.language
    ));

    s.push_str("Environment:\n");
    s.push_str(&format!("- today: {}\n", env.date));
    s.push_str(&format!("- system: {}\n", env.os));
    s.push_str(&format!("- working directory: {}\n", env.cwd));

    let mut listed = env.tools.clone();
    if !env.pool.is_empty() {
        listed.push("autotools".to_string());
    }
    if listed.is_empty() {
        s.push_str(
            "- tools are disabled: answer from your own knowledge and say \
             honestly when you lack the data.\n",
        );
    } else {
        s.push_str(&format!("- available tools: {}\n", listed.join(", ")));
        s.push_str(&format!(
            "\nRules for working with tools (budget — {} turns per user \
             message):\n",
            env.max_turns
        ));
        s.push_str(
            "1. Every call must yield a new fact. Don't repeat a call \
             you've already made in this dialogue: the result will be the \
             same, and you'll have spent a turn for nothing.\n\
             2. Didn't work — change your approach: a different command, a \
             different path, a different tool. Repeating the exact same \
             thing is a dead end, not an attempt.\n\
             3. Rely on results already present earlier in the dialogue; \
             no need to re-read the same thing.\n\
             4. As soon as you have enough data to answer — stop calling \
             tools and reply with text. Exploring \"just in case\" wastes \
             the turn budget.\n\
             5. If the task can't be solved with the tools available — say \
             so: what you found, where you got stuck, what you suggest \
             next. Silently going in circles is not allowed.\n\
             6. Before an irreversible action (modifying or deleting \
             files, installing packages, network requests with side \
             effects), state in one sentence what you're about to do.\n\
             7. Tool arguments are JSON values, not text. Pass an array or \
             an object as a real structure — \"set_state\": [{...}] — never \
             as a quoted string with escaped JSON inside, and never glue \
             several arguments into one string. Long values are where \
             brackets go wrong: check that every one you opened is closed, \
             in the right order, before you send the call.\n\
             8. Announced an action — perform it in this same turn. A text \
             reply ends the turn: \"I'll look at the project now\" without \
             an actual call means nothing happened and the user is left \
             waiting. Either call the tool, or don't announce it.\n",
        );
        // Правила инструментов — только для активных: текст зависит лишь от
        // набора инструментов (как и строка со списком), поэтому префикс-KV
        // между сообщениями не страдает. Инструменту из пула правила приходят
        // вместе со схемой в ответе `autotools`.
        for key in RULED_TOOLS {
            if env.tools.iter().any(|t| t == key) {
                s.push_str(tool_rules(key).unwrap_or_default());
            }
        }
        if !env.pool.is_empty() {
            s.push_str(&autotools_rules(&env.pool));
        }
    }

    s.push_str(
        "\nAnswer: to the point, no filler, and no step-by-step recap of \
         your own actions. Code and paths must be exact, as in the source.",
    );

    let extra = env.user_prompt.trim();
    if !extra.is_empty() {
        s.push_str("\n\nAdditional user instructions:\n");
        s.push_str(extra);
    }
    s
}

/// Инструменты со своим блоком правил — в порядке, в каком блоки идут в
/// промпте.
const RULED_TOOLS: [&str; 3] = ["pipelines", "wizard", "notes"];

/// Блок правил инструмента: в системный промпт — когда он активен, в ответ
/// `autotools` — когда модель подгружает его из пула. Текст начинается с
/// перевода строки: так он ложится в промпт следом за общими правилами.
pub fn tool_rules(key: &str) -> Option<&'static str> {
    Some(match key {
        "pipelines" => PIPELINES_RULES,
        "wizard" => WIZARD_RULES,
        "notes" => NOTES_RULES,
        _ => return None,
    })
}

/// Строка про пул `autotools`. Пул меняет только пользователь, поэтому текст
/// стабилен от хода к ходу и префикс-KV не рвёт.
fn autotools_rules(pool: &[String]) -> String {
    format!(
        "\nAutotools (`autotools`) — tools you can load on demand: {}. Their \
         descriptions are not in your context. When the task needs one, call \
         `autotools` with its id first, then call the tool itself by name with \
         the parameters from the reply. Don't guess the parameters of a tool \
         you haven't loaded, and don't load a tool again once its description \
         is already in this dialogue.\n",
        pool.join(", ")
    )
}

const PIPELINES_RULES: &str =
    "\nPipelines (`pipelines`) — video/music/speech generation \
     in the node editor:\n\
     - Any audio/music/voice/video task (extract vocals or other \
     stems from a song, cover, extend or edit a track, music from \
     text, TTS, voice clone, transcription, video) — do it with \
     pipelines right away: start with list and pick the template. \
     Don't search the disk or the web for external tools \
     (demucs, spleeter, ffmpeg, pip) unless the user asks for them.\n\
     - Order: list → open (template) → nodes with filter (the \
     exact schema of the nodes you need) → apply (prompts, \
     parameters, ref inputs) → run. Compose the \
     scenario/prompt yourself and put it in the nodes' state.\n\
     - nodes: in filter list the nodes themselves (space-\
     separated), not field names — fields are visible in the \
     state example. A filter miss returns a list of node \
     kinds: take names from there rather than guessing.\n\
     - Before a heavy run (LTX, MiniMax-H3), check `system \
     status`: if the pipeline's models don't fit alongside \
     you — run with free_vram=true (you'll be unloaded for the \
     duration of the run and restored; this doesn't harm the \
     history).\n\
     - run blocks the turn until it finishes and can take tens \
     of minutes — that's normal, the user sees a live status. \
     Results (video/audio) are attached to the message \
     automatically.\n\
     - Don't make up model paths: the directory inventory \
     (main dir and HF cache) is in pipelines list — the \
     \"Models in directory\" sections; fill empty checkpoint \
     model_path values via apply set_state before run. \
     Numeric `*_idx` state fields (quant, device, aspect) are \
     decoded in action=nodes under the state example.\n\
     - Re-running the same graph — with a new run_id.\n\
     - Once you've announced an action, do it in this same \
     turn: a text reply ends the turn, and \"I'll run it now\" \
     without calling run means the run never happened.\n\
     - Wire ref images from the chat via the \
     attachment:<name|sha|last> reference in state paths (list \
     — in pipelines list).\n";

const WIZARD_RULES: &str =
    "\nWizard (`wizard`) — a question panel for the user: \
     clickable options (single or multiple choice) and/or a \
     free-text field. Use it when a decision is the user's to \
     make — a period, a template, confirm vs alternatives — \
     instead of asking in plain text. The call ENDS your turn: \
     write nothing after it and call no other tool in that \
     turn; the answer arrives as the user's next message (the \
     option labels, or the typed text). One question per call, \
     2–6 short options. The question is Markdown: put code in a \
     fenced block with a language tag (```rust … ```), never \
     indented.\n";

const NOTES_RULES: &str =
    "\nNotes (`notes`) — the user's notebook in the app: a tree of \
     markdown pages (each page is a free canvas) with shapes, \
     arrows, kanban boards, Gantt charts, mind maps, calendars \
     and charts (line, bar, pie, radar, gauge) embedded in \
     them.\n\
     - Start with list (page tree, ids, the boards/charts on each \
     page) or search; read a page before editing it — update \
     with content and mode=replace overwrites the whole page.\n\
     - Read in bulk, not page by page: read takes pages=[…] \
     (several pages at once), depth=all (the page with its \
     whole subtree) and page=\"all\" (the entire project), and \
     blocks op=read block=all returns every block of a page \
     with its markdown and attributes. One call brings back \
     everything that fits in a reply and lists by id whatever \
     did not — a call per page or per block only wastes the \
     user's time.\n\
     - Address pages by id (12 hex) or exact title; an ambiguous \
     title is an error — use the id. Boards and charts: by id \
     from list/read, or implicitly when the page has one. \
     Blocks: by index from read blocks=true / blocks op=list \
     (re-list after inserting or deleting — indices shift) or \
     find:<text>.\n\
     - Small edits: update with find/replace (an exact markdown \
     fragment from read), mode=append, or blocks op=set_markdown \
     / insert / delete / move — these keep the canvas position \
     and style of untouched blocks. Rewrite a whole page only \
     when the user asks for it.\n\
     - Canvas: coordinates are px from the page's top-left; a \
     block with x y is pinned there, without them it flows in \
     the column. blocks op=pin / move place blocks (x y, w, h), \
     op=set_attrs styles them (color, bg, size, align, weight). \
     update sets the page grid (none|dots|lines|cross, \
     grid_step) and snap (snap, snap_step).\n\
     - Shapes and arrows: shape op=create kind=rect|ellipse|\
     triangle|diamond with x y w h and fill/stroke/sw/dash/\
     radius/opacity; lines, arrows and curves take absolute end \
     points x1 y1 x2 y2. shape op=connect from=<block> \
     to=<block> draws an arrow between two pinned blocks (pin \
     them first). A diagram = pinned text blocks + connect.\n\
     - Life management — one call, not a tour of the project: \
     agenda (overdue, due today/tomorrow/soon, events, recently \
     done, board counts) answers \"what should I do / how is my \
     day or week\"; tasks {due=overdue|today|week|none, tag, \
     priority, done} finds cards across every board; log \
     {since=7d|yyyy-mm-dd, board, card} tells what was added, \
     moved, done, archived, when and by whom (the app records it \
     itself — never keep a history page by hand, never write \
     \"done on …\" into card text); journal {date=today, content} \
     appends to the day page (Journal / yyyy-mm / yyyy-mm-dd; a \
     [[yyyy-mm-dd]] link opens it).\n\
     - Boards: kanban op=create on a page (columns optional; \
     index/after/before and x y w h place it), then add_card \
     {column, title, md, priority, tags, due, repeat} — the new \
     card's text is title; update_card / move_card / delete_card \
     / archive take card=<id or exact title of an EXISTING card>. \
     Cards carry a title, a markdown \
     body (a `- [ ]` checklist shows progress), priority \
     low|medium|high|urgent, tags, a due date yyyy-mm-dd, repeat \
     daily|weekly|monthly|yearly (closing spawns the next one) and \
     attachments (kanban op=attach card path|attachment: images \
     become thumbnails, files get a paperclip; the file goes into \
     the project — never paste file bytes). One column is the DONE \
     column (read shows \"DONE column\"; set it with \
     update_column done=true): a task \"done\" = move_card there \
     — the card gets its done date automatically; done cards \
     leave the board into the archive after set_style \
     archive_after days (read archived=true, op=unarchive). \
     op=set_style also sets column_width, lane_bg, card_bg, \
     show_counts.\n\
     - Planning a card in time: duration is the estimate (1d, 2h, \
     90m), start/end are the planned dates; kanban op=schedule \
     {card, date=today|tomorrow|yyyy-mm-dd, time, duration} fills \
     them in one call (op=unschedule clears them). A card with a \
     plan shows up as a BAR in the calendar (day and week views \
     place it by its hours) and as a row on a Gantt chart, and \
     agenda lists it under Today. A due date alone is only a \
     point — plan the work with start+duration.\n\
     - Charts: gantt op=create, add_task with start/end \
     yyyy-mm-dd (after=<task> adds a dependency), update_task, \
     add_dep, set_zoom; set_boards {boards} makes the planned \
     cards of those boards rows of the same chart.\n\
     - Mind maps: mindmap op=create on a page (outline=<markdown \
     list> builds the whole map at once — a heading is the root, \
     nested bullets are nodes), then add_node (parent=<node|root>), \
     update_node (text, note, link=<page>, color, shape, icon, \
     collapsed), move_node, add_link for cross links, set_layout \
     (direction right|left|both|down|radial, curve, gaps) and \
     set_style. from_list turns an existing list block into a map.\n\
     - Calendar: events live in one project-wide store, a \
     calendar:<id> widget on a page is just a view (year | month | \
     week | day) over them; card due dates and Gantt tasks are \
     shown on it by themselves — never duplicate them as events. \
     calendar op=add_event with title and \
     date yyyy-mm-dd (start_time/end_time HH:MM local, all_day, \
     repeat daily|weekly|monthly|yearly + until, calendar=<name>, \
     note, link=<page>); list_events from/to reads a range; \
     update_event / move_event / complete / delete_event by id or \
     title; add_calendar makes a named colour category; op=create \
     puts a widget on a page, set_view and set_style change it \
     (set_view boards=[…] picks which boards' tasks the widget \
     shows; style show_kanban_spans/show_kanban_due/show_gantt \
     turn the layers off). Deleting a widget keeps the events.\n\
     - Changes are saved automatically and show up in the UI at \
     once; open shows a page to the user. Don't ask to confirm \
     routine edits the user already requested.\n";

/// Заметка про остаток бюджета ходов. Дописывается к результату инструмента
/// (а не в system-сообщение) намеренно: system лежит в голове контекста, и
/// правка на каждом ходу обнуляла бы префикс-KV, а хвост промпта и так
/// пересчитывается.
pub fn budget_note(turns_left: usize) -> String {
    format!(
        "\n\n[System note: agent turns remaining — {turns_left}. \
         Wrap up and give a text answer based on what's already known.]"
    )
}

/// Порог, начиная с которого к результатам инструментов дописывается
/// [`budget_note`]. Раньше смысла нет: заметка на каждом ходу — это шум.
pub const BUDGET_NOTE_FROM: usize = 3;

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> PromptEnv {
        PromptEnv {
            date: "2026-08-19".to_string(),
            os: "linux (x86_64)".to_string(),
            cwd: "/home/u/proj".to_string(),
            tools: vec!["bash".to_string(), "web".to_string()],
            pool: Vec::new(),
            max_turns: 32,
            user_prompt: String::new(),
            language: "Русский".to_string(),
        }
    }

    /// Язык ответа назван прямым текстом: «reply in the user's language»
    /// на английском промпте модель читала как «отвечай по-английски».
    #[test]
    fn build_names_answer_language_explicitly() {
        let s = build(&env());
        assert!(s.contains("answer in Русский"), "{s}");
    }

    /// Объявил действие — сделай его в этом же ходе. Ровно на этом ход
    /// заканчивался текстом «I'll start by exploring the project» без
    /// единого вызова.
    #[test]
    fn build_forbids_announcing_without_doing() {
        let s = build(&env());
        assert!(s.contains("Announced an action — perform it in this same turn"), "{s}");
    }

    #[test]
    fn build_includes_environment_and_rules() {
        let s = build(&env());
        assert!(s.contains("2026-08-19"));
        assert!(s.contains("/home/u/proj"));
        assert!(s.contains("bash, web"));
        assert!(s.contains("32 turns"));
        assert!(s.contains("Every call must yield a new fact"));
    }

    /// Правило про структурные аргументы — общее для всех инструментов,
    /// а не часть pipelines-блока: строкой аргументы шлют и в bash, и в web.
    #[test]
    fn build_demands_structured_tool_arguments() {
        let s = build(&env());
        assert!(s.contains("Tool arguments are JSON values, not text"), "{s}");
        assert!(s.contains("never as a quoted string"), "{s}");
    }

    #[test]
    fn build_has_no_clock_time_for_prefix_kv_stability() {
        // Промпт лежит в голове контекста: любое ежеминутно меняющееся поле
        // обнуляло бы переиспользование префикс-KV между сообщениями.
        let s = build(&env());
        assert!(!s.to_lowercase().contains("time:"));
    }

    #[test]
    fn build_without_tools_drops_tool_rules() {
        let mut e = env();
        e.tools.clear();
        let s = build(&e);
        assert!(s.contains("tools are disabled"));
        assert!(!s.contains("Every call must yield a new fact"));
    }

    #[test]
    fn user_prompt_is_appended_not_replacing() {
        let mut e = env();
        e.user_prompt = "  Answer briefly.  ".to_string();
        let s = build(&e);
        assert!(s.contains("Every call must yield a new fact"));
        assert!(s.trim_end().ends_with("Answer briefly."));
    }

    #[test]
    fn budget_note_mentions_remaining_turns() {
        assert!(budget_note(2).contains("agent turns remaining — 2"));
    }

    #[test]
    fn wizard_rules_only_with_wizard_tool() {
        let s = build(&env());
        assert!(!s.contains("Wizard (`wizard`)"));
        let mut e = env();
        e.tools.push("wizard".to_string());
        let s = build(&e);
        assert!(s.contains("Wizard (`wizard`)") && s.contains("ENDS your turn"), "{s}");
    }

    #[test]
    fn notes_rules_only_with_notes_tool() {
        let s = build(&env());
        assert!(!s.contains("Notes (`notes`)"));
        let mut e = env();
        e.tools.push("notes".to_string());
        let s = build(&e);
        assert!(s.contains("Notes (`notes`)"), "{s}");
        assert!(s.contains("find/replace"), "{s}");
        assert!(s.contains("kanban op=create"), "{s}");
        assert!(s.contains("add_card {column, title, md, priority, tags, due, repeat}"), "{s}");
        assert!(s.contains("shape op=connect"), "{s}");
        assert!(s.contains("blocks op=pin"), "{s}");
        assert!(s.contains("mindmap op=create"), "{s}");
        assert!(s.contains("calendar op=add_event"), "{s}");
        assert!(s.contains("depth=all"), "{s}");
        assert!(s.contains("agenda"), "{s}");
        assert!(s.contains("never keep a history page by hand"), "{s}");
        assert!(s.contains("kanban op=attach"), "{s}");
    }

    #[test]
    fn pipeline_rules_only_with_pipelines_tool() {
        let s = build(&env());
        assert!(!s.contains("Pipelines"));
        let mut e = env();
        e.tools.push("pipelines".to_string());
        let s = build(&e);
        assert!(s.contains("Pipelines"));
        assert!(s.contains("free_vram"));
        assert!(s.contains("attachment:"));
    }

    /// Пул `autotools`: в строке инструментов — сам `autotools`, в правилах —
    /// что можно подгрузить. Правил инструмента из пула в промпте нет: они
    /// приходят ответом `autotools` вместе со схемой.
    #[test]
    fn pool_is_named_but_its_rules_stay_out() {
        let mut e = env();
        e.pool = vec!["notes".to_string(), "pipelines".to_string()];
        let s = build(&e);
        assert!(s.contains("available tools: bash, web, autotools"), "{s}");
        assert!(s.contains("tools you can load on demand: notes, pipelines"), "{s}");
        assert!(!s.contains("Notes (`notes`)"), "{s}");
        assert!(!s.contains("Pipelines (`pipelines`)"), "{s}");
        assert!(!build(&env()).contains("autotools"));
    }

    #[test]
    fn pool_alone_keeps_tools_enabled() {
        let mut e = env();
        e.tools.clear();
        e.pool = vec!["web".to_string()];
        let s = build(&e);
        assert!(!s.contains("tools are disabled"), "{s}");
        assert!(s.contains("available tools: autotools"), "{s}");
        assert!(s.contains("Every call must yield a new fact"), "{s}");
    }

    /// Блок правил в промпте активного инструмента и в ответе `autotools` —
    /// один и тот же текст.
    #[test]
    fn tool_rules_match_prompt_blocks() {
        for key in RULED_TOOLS {
            let mut e = env();
            e.tools.push(key.to_string());
            assert!(build(&e).contains(tool_rules(key).unwrap()), "{key}");
        }
        assert!(tool_rules("bash").is_none());
    }
}
