//! Замер ленты чата (стадия S0 плана ускорения ленты): синтетический чат на
//! 70 и 300 сообщений в настоящей ленте (`message_area::view`), полный кадр
//! приложения [`TestHarness::frame`] и счётчики [`syngui::perf::counters`].
//! По сценариям — открытие чата, агентный шаг (tool_call + tool_result),
//! 20 сбросов стрима, прокрутка колесом — печатает время фаз кадра и дельты
//! счётчиков. Запуск:
//! `cargo test -p synthos --features testing --lib feed_bench -- --ignored --nocapture --test-threads=1`.
//!
//! Без GPU нет атласа шрифтов: текст меряет [`Proportional`] — ширина по
//! числу символов. Раскладка поэтому дешевле живой (нет шейпинга и поиска
//! глифов), а число элементов, пересборок, разборов и подсветок — то же, что
//! в приложении. Автосейв ленты установлен, как в приложении: в фазе
//! эффектов он только отмечает правку, а таймер записи в тестах не
//! взводится. `HOME` на время замера — временный каталог: конфиг,
//! пресеты и файлы чатов, которые пишет автосейв, настоящих не трогают.

use std::sync::Arc;
use std::time::{Duration, Instant};

use syngui::core::Point;
use syngui::input::Event;
use syngui::mss::{parse_stylesheet_str, StyleEngine};
use syngui::perf::counters::{self, Snapshot};
use syngui::prelude::*;
use syngui::testing::{FrameTimings, TestHarness};
use syngui::widget::context::TextMeasure;
use syngui::widget::{Element, ElementId};

use crate::agent::schema::{ChatToolCall, ChatToolCallFunction};
use crate::agent::state::ChatMeta;
use crate::context::AppCtx;
use crate::syn_chat::state::{ChatMsg, ChatMsgKind, ChatMsgRole, SynChatCtx};

use super::message_area;

/// Центральная колонка развёрнутого окна: лента между панелями.
const VIEW_W: f32 = 960.0;
const VIEW_H: f32 = 820.0;
const CHAT_ID: &str = "feed-bench";
const CHAT_TITLE: &str = "Замер ленты";

/// Измеритель без атласа шрифтов: средняя ширина глифа — 0,55 кегля.
struct Proportional;

impl TextMeasure for Proportional {
    fn measure_text_width(&self, _text: &str, font_size: f32, chars: usize) -> f32 {
        chars as f32 * font_size * 0.55
    }
    fn hit_test_char(&self, text: &str, font_size: f32, x: f32) -> usize {
        ((x / (font_size * 0.55)).round().max(0.0) as usize).min(text.chars().count())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Синтетический чат
// ─────────────────────────────────────────────────────────────────────────────

/// xorshift64*: детерминированный и без зависимостей — один и тот же чат
/// на каждом прогоне, числа «до» и «после» сравнимы.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() % (hi - lo + 1) as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<'a>(&mut self, xs: &'a [&'a str]) -> &'a str {
        xs[self.range(0, xs.len() - 1)]
    }
    fn word(&mut self) -> &'static str {
        WORDS[self.range(0, WORDS.len() - 1)]
    }
}

const WORDS: &[&str] = &[
    "модель", "контекст", "слой", "буфер", "лента", "кадр", "элемент", "сигнал", "очередь",
    "поток", "раскладка", "пересборка", "окно", "ключ", "кэш", "ответ", "запрос", "файл", "путь",
    "быстро", "заметно", "всегда", "иногда", "сначала", "потом", "каждый", "весь", "новый",
    "старый", "проверка", "ошибка", "результат", "память", "таблица", "layout", "render", "token",
    "prefill", "decode", "tensor", "kernel", "batch", "stream", "cache",
];

fn sentence(rng: &mut Rng) -> String {
    let n = rng.range(6, 16);
    let mut s = String::new();
    for i in 0..n {
        let w = rng.word();
        if i == 0 {
            let mut cs = w.chars();
            if let Some(c) = cs.next() {
                s.extend(c.to_uppercase());
                s.push_str(cs.as_str());
            }
        } else {
            s.push(' ');
            s.push_str(w);
        }
    }
    s.push('.');
    s
}

fn paragraph(rng: &mut Rng, lo: usize, hi: usize) -> String {
    (0..rng.range(lo, hi)).map(|_| sentence(rng)).collect::<Vec<_>>().join(" ")
}

fn bullet_list(rng: &mut Rng) -> String {
    (0..rng.range(3, 6))
        .map(|_| format!("- **{}** — {}", rng.word(), sentence(rng)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn table(rng: &mut Rng) -> String {
    let mut s = String::from("| Параметр | До | После |\n|---|---:|---:|\n");
    for _ in 0..rng.range(4, 6) {
        s.push_str(&format!("| {} | {} | {} |\n", rng.word(), rng.range(1, 999), rng.range(1, 999)));
    }
    s
}

fn code_line(rng: &mut Rng, lang: &str, i: usize) -> String {
    let w = rng.word();
    let k = rng.range(1, 999);
    match lang {
        "rust" => match i % 7 {
            0 => format!("fn {w}_{i}(input: &[u8], k: usize) -> Result<Vec<u32>, Error> {{"),
            1 => format!("    let {w} = input.iter().map(|b| *b as u32 * {k}).collect::<Vec<_>>();"),
            2 => format!("    if {w}.len() > k {{ return Err(Error::TooLong({k})); }}"),
            3 => format!("    // {} {} {}", rng.word(), rng.word(), rng.word()),
            4 => format!("    let total: u32 = {w}.iter().sum::<u32>() + {k};"),
            5 => format!("    Ok({w})"),
            _ => "}".to_string(),
        },
        "python" => match i % 5 {
            0 => format!("def {w}_{i}(data, k={k}):"),
            1 => format!("    result = [x * {k} for x in data if x % {} == 0]", k % 7 + 2),
            2 => format!("    # {} {} {}", rng.word(), rng.word(), rng.word()),
            3 => format!("    print(f\"{w}: {{len(result)}}\")"),
            _ => "    return result".to_string(),
        },
        "bash" => match i % 4 {
            0 => format!("cargo build --release -p {w} 2>&1 | tail -n {k}"),
            1 => format!("export SYN_{}={k}", w.to_uppercase()),
            2 => format!("# {} {}", rng.word(), rng.word()),
            _ => format!("ls -la ~/proj/{w}/{k} | grep -v target"),
        },
        _ => match i % 3 {
            0 => format!("  \"{w}_{i}\": {k},"),
            1 => format!("  \"{w}\": \"{} {}\",", rng.word(), rng.word()),
            _ => format!("  \"list_{i}\": [{k}, {}, {}],", k / 2, k / 3),
        },
    }
}

fn code_block(rng: &mut Rng, lines: usize) -> String {
    let lang = rng.pick(&["rust", "python", "bash", "json"]);
    let mut s = format!("```{lang}\n");
    for i in 0..lines {
        s.push_str(&code_line(rng, lang, i));
        s.push('\n');
    }
    s.push_str("```");
    s
}

fn user_msg(rng: &mut Rng) -> ChatMsg {
    let mut body = paragraph(rng, 1, 3);
    if rng.chance(30) {
        body.push_str(&format!(" Посмотри `{}::{}`.", rng.word(), rng.word()));
    }
    if rng.chance(15) {
        let lines = rng.range(5, 12);
        body.push_str("\n\n");
        body.push_str(&code_block(rng, lines));
    }
    ChatMsg::user(body)
}

fn answer_body(rng: &mut Rng) -> String {
    let mut parts = vec![paragraph(rng, 2, 4)];
    if rng.chance(60) {
        parts.push(bullet_list(rng));
    }
    if rng.chance(70) {
        let lines = rng.range(20, 80);
        parts.push(code_block(rng, lines));
    }
    if rng.chance(20) {
        parts.push(table(rng));
    }
    parts.push(paragraph(rng, 1, 3));
    parts.join("\n\n")
}

fn assistant_msg(rng: &mut Rng) -> ChatMsg {
    let mut m = ChatMsg::assistant_empty();
    if rng.chance(70) {
        m.thinking = (0..rng.range(1, 3))
            .map(|_| paragraph(rng, 2, 5))
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    m.body = answer_body(rng);
    m
}

fn tool_output(rng: &mut Rng, tool: &str) -> String {
    let lines = rng.range(30, 150);
    (0..lines)
        .map(|i| {
            let w = rng.word();
            let k = rng.range(1, 9999);
            match tool {
                "bash" if i % 3 == 0 => format!("   Compiling {w}-{k} v0.{i}.0 (/home/user/proj/{w})"),
                "bash" => format!("test {w}::{}_{i} ... ok ({k} ms)", rng.word()),
                "web_read" => sentence(rng),
                _ => format!("[{k}] {w}/{}.md: {}", rng.word(), sentence(rng)),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn tool_pair(rng: &mut Rng, no: usize) -> (ChatMsg, ChatMsg) {
    let tool = rng.pick(&["bash", "web_read", "kb_search"]);
    let (w1, w2, k) = (rng.word(), rng.word(), rng.range(1, 99));
    let args = match tool {
        "bash" => format!("{{\n  \"command\": \"cargo test -p {w1} --lib {w2}\",\n  \"timeout\": {k}\n}}"),
        "web_read" => format!("{{\n  \"url\": \"https://example.com/{w1}/{w2}/{k}\"\n}}"),
        _ => format!("{{\n  \"query\": \"{w1} {w2}\",\n  \"top_k\": {}\n}}", k % 10 + 1),
    };
    let id = format!("call-{no}");
    let call = ChatToolCall {
        id: id.clone(),
        kind: "function".to_string(),
        function: ChatToolCallFunction {
            name: Some(tool.to_string()),
            arguments: Some(args.replace('\n', "")),
        },
    };
    let error = rng.chance(5);
    (
        ChatMsg::tool_call(tool, args, vec![call]),
        ChatMsg::tool_result(id, tool, tool_output(rng, tool), error),
    )
}

/// Лента из `n` сообщений: вопрос, у половины ходов — 1–3 пары
/// tool_call/tool_result с длинным выводом, ответ с размышлением, списками и
/// блоками кода на 20–80 строк. Кончается ответом ассистента.
fn synthetic_chat(n: usize, seed: u64) -> Vec<ChatMsg> {
    let mut rng = Rng::new(seed);
    let mut out = Vec::with_capacity(n + 8);
    let mut call_no = 0;
    while out.len() < n {
        out.push(user_msg(&mut rng));
        if rng.chance(55) {
            for _ in 0..rng.range(1, 3) {
                call_no += 1;
                let (call, result) = tool_pair(&mut rng, call_no);
                out.push(call);
                out.push(result);
            }
        }
        out.push(assistant_msg(&mut rng));
    }
    out.truncate(n);
    if let Some(last) = out.last_mut() {
        if last.role != ChatMsgRole::Assistant || !matches!(last.kind, ChatMsgKind::Text) {
            *last = assistant_msg(&mut rng);
        }
    }
    out
}

fn describe(msgs: &[ChatMsg]) -> String {
    let count = |f: &dyn Fn(&ChatMsg) -> bool| msgs.iter().filter(|m| f(m)).count();
    let users = count(&|m| m.role == ChatMsgRole::User);
    let answers = count(&|m| m.role == ChatMsgRole::Assistant && matches!(m.kind, ChatMsgKind::Text));
    let calls = count(&|m| matches!(m.kind, ChatMsgKind::ToolCall { .. }));
    let results = count(&|m| matches!(m.kind, ChatMsgKind::ToolResult { .. }));
    let thinking = count(&|m| !m.thinking.is_empty());
    let fences: usize = msgs.iter().map(|m| m.body.matches("```").count()).sum();
    let bytes: usize = msgs.iter().map(|m| m.body.len() + m.thinking.len()).sum();
    format!(
        "{} сообщ.: user {users}, ответов {answers} (с размышлением {thinking}), tool_call {calls}, \
         tool_result {results}, блоков кода {}, текста {:.0} КБ",
        msgs.len(),
        fences / 2,
        bytes as f64 / 1024.0
    )
}

/// Разбить текст на `k` кусков по границам символов — сбросы стрима.
fn chunks(s: &str, k: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let step = chars.len().div_ceil(k);
    chars.chunks(step).map(|c| c.iter().collect()).collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Замер
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Sample {
    wall: Duration,
    t: FrameTimings,
    d: Snapshot,
    live: usize,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

struct World {
    h: TestHarness,
    engine: StyleEngine,
    ctx: SynChatCtx,
    n: usize,
    print: bool,
}

impl World {
    fn new(ctx: SynChatCtx, n: usize, print: bool) -> Self {
        let mut h = TestHarness::new(Box::new(
            Column::new()
                .expand()
                .child(DecoratedBox::new().class("grow").child(message_area::view())),
        ));
        h.tree.text_measure = Some(Arc::new(Proportional));
        let app = use_context::<AppCtx>();
        let mut engine =
            StyleEngine::new(parse_stylesheet_str(crate::styles::styles()).expect("styles.mss"));
        engine.load_additional_stylesheet(
            parse_stylesheet_str(&app.theme_mss.get_untracked()).expect("MSS темы"),
        );
        // Как при сборке корня в приложении: стили всему дереву один раз,
        // дальше — только новым элементам внутри кадра.
        h.apply_styles(&engine);
        Self { h, engine, ctx, n, print }
    }

    /// Действие над деревом/состоянием и один кадр после него.
    fn step(&mut self, label: &str, act: impl FnOnce(&mut TestHarness, &SynChatCtx)) -> Sample {
        let s = self.sample(act);
        if self.print {
            self.line(label, &s);
        }
        s
    }

    fn sample(&mut self, act: impl FnOnce(&mut TestHarness, &SynChatCtx)) -> Sample {
        let before = counters::snapshot();
        let started = Instant::now();
        act(&mut self.h, &self.ctx);
        let t = self.h.frame(Some(&self.engine), VIEW_W, VIEW_H);
        let wall = started.elapsed();
        let d = counters::snapshot().since(&before);
        Sample { wall, t, d, live: self.h.element_count() }
    }

    fn line(&self, label: &str, s: &Sample) {
        let t = &s.t;
        println!(
            "N={:<3} {label:<24} wall {:>8.2} мс | кадр {:>8.2} = пересб {:>7.2} стили {:>7.2} эфф {:>6.2} раскл {:>7.2}{} a11y {:>6.2} отрис {:>6.2} | {} | живых {} | cmds {}",
            self.n,
            ms(s.wall),
            ms(t.total()),
            ms(t.rebuild),
            ms(t.styles),
            ms(t.effects),
            ms(t.layout),
            if t.relayout { "*" } else { " " },
            ms(t.a11y),
            ms(t.paint),
            s.d,
            s.live,
            t.commands,
        );
    }

    fn aggregate(&self, label: &str, samples: &[Sample]) {
        if !self.print || samples.is_empty() {
            return;
        }
        let k = samples.len() as f64;
        let totals: Vec<f64> = samples.iter().map(|s| ms(s.t.total())).collect();
        let avg = |f: &dyn Fn(&FrameTimings) -> Duration| {
            samples.iter().map(|s| ms(f(&s.t))).sum::<f64>() / k
        };
        let sum = samples.iter().fold(Snapshot::default(), |a, s| a.plus(&s.d));
        println!(
            "N={:<3} {label:<24} кадр ср {:>7.2} мин {:>7.2} макс {:>7.2} мс | ср: пересб {:.2} стили {:.2} эфф {:.2} раскл {:.2} a11y {:.2} отрис {:.2} | сумма за {}: {}",
            self.n,
            totals.iter().sum::<f64>() / k,
            totals.iter().cloned().fold(f64::INFINITY, f64::min),
            totals.iter().cloned().fold(0.0, f64::max),
            avg(&|t| t.rebuild),
            avg(&|t| t.styles),
            avg(&|t| t.effects),
            avg(&|t| t.layout),
            avg(&|t| t.a11y),
            avg(&|t| t.paint),
            samples.len(),
            sum,
        );
    }

    /// Прокрутчик ленты — первый `ScrollView` в обходе (внешний).
    fn feed_scroll(&self) -> ElementId {
        *self
            .h
            .find_by_type_name("ScrollView")
            .first()
            .expect("лента не смонтировала ScrollView")
    }

    fn scroll_y(&self) -> f32 {
        let id = self.feed_scroll();
        self.h.tree.get(id).map(|e| e.scroll_offset().y).unwrap_or(0.0)
    }
}

/// Открытие чата — как `registry::select_internal`: лента, параметры и
/// отпечаток автосейва (иначе открытие выглядело бы для него правкой).
fn open_chat(ctx: &SynChatCtx, msgs: Vec<ChatMsg>) {
    ctx.loading.set(true);
    ctx.active_chat_id.set(Some(CHAT_ID.to_string()));
    ctx.messages.set(msgs.clone());
    let params = ctx.params.get_untracked();
    let title = CHAT_TITLE.to_string();
    ctx.last_saved_fp
        .set(crate::syn_chat::registry::state_fingerprint(&title, &msgs, &params));
    ctx.highlight_msg.set(None);
    ctx.editing_msg.set(None);
    ctx.streaming_body.set(String::new());
    ctx.streaming_thinking.set(String::new());
    ctx.streaming_tool.set(String::new());
    ctx.pending.set(false);
    ctx.loading.set(false);
}

fn close_chat(ctx: &SynChatCtx) {
    ctx.active_chat_id.set(None);
    ctx.messages.set(Vec::new());
    ctx.pending.set(false);
    ctx.streaming_body.set(String::new());
}

fn run(n: usize, print: bool) {
    let ctx = use_context::<SynChatCtx>();
    close_chat(&ctx);
    syngui::signal::drain_and_run_effects();
    let msgs = synthetic_chat(n, 0x5EED_0000 ^ n as u64);
    let mut rng = Rng::new(0xA11CE ^ n as u64);
    if print {
        println!("\n=== N={n}: {}", describe(&msgs));
    }
    let mut w = World::new(ctx, n, print);
    w.step("монтаж (нет чата)", |_, _| {});

    // (a) Открытие чата и кадры после него.
    let open = w.step("открытие чата", |_, ctx| open_chat(ctx, msgs));
    w.step("открытие +1 кадр", |_, _| {});
    w.step("простой кадр", |_, _| {});
    let scroll_id = w.feed_scroll();
    let viewport = w.h.element_bounds(scroll_id);
    let content = w
        .h
        .tree
        .children_of(scroll_id)
        .first()
        .map(|&c| w.h.element_bounds(c).size.height)
        .unwrap_or(0.0);
    let bubbles = w.h.find_by_type_name("MarkdownView").len();
    if print {
        println!(
            "N={n:<3} лента: ScrollView {:.0}×{:.0}, содержимое {:.0} px, MarkdownView {bubbles}, элементов {}",
            viewport.size.width, viewport.size.height, content, open.live
        );
    }
    assert!(bubbles > 0, "лента не построила пузырьки");
    assert!(
        viewport.size.height <= VIEW_H + 1.0,
        "ScrollView вырос до содержимого — замер не про прокручиваемую ленту: {viewport:?}"
    );

    // (b) Агентный ход: вопрос и заглушка, вызов инструмента на месте
    // заглушки, результат, заглушка следующего шага (`session.rs`).
    let question = user_msg(&mut rng);
    w.step("ход: user + заглушка", move |_, ctx| {
        ctx.messages.update(|m| {
            m.push(question);
            m.push(ChatMsg::assistant_empty());
        });
        ctx.pending.set(true);
    });
    let (call, result) = tool_pair(&mut rng, 10_000);
    w.step("tool_call", move |_, ctx| {
        ctx.messages.update(|m| {
            if m.last().is_some_and(|x| x.role == ChatMsgRole::Assistant && x.body.is_empty()) {
                m.pop();
            }
            m.push(call);
        });
    });
    w.step("tool_result", move |_, ctx| ctx.messages.update(|m| m.push(result)));
    w.step("заглушка след. шага", |_, ctx| {
        ctx.messages.update(|m| m.push(ChatMsg::assistant_empty()))
    });

    // (c) Стрим ответа: 20 сбросов `flush_streaming` в хвост ленты.
    let answer = format!(
        "{}\n\n{}\n\n{}\n\n{}",
        paragraph(&mut rng, 2, 3),
        bullet_list(&mut rng),
        code_block(&mut rng, 40),
        paragraph(&mut rng, 1, 2)
    );
    let mut stream = Vec::new();
    let parts = chunks(&answer, 20);
    let last = parts.len() - 1;
    for (i, chunk) in parts.into_iter().enumerate() {
        let s = w.sample(move |_, ctx| {
            ctx.streaming_body.update(|b| b.push_str(&chunk));
            ctx.last_gen_tokens.set_always(i as u32 * 12);
            ctx.last_decode_tps.set_always(40.0);
        });
        if print && (i == 0 || i == last) {
            w.line(&format!("стрим: сброс {}", i + 1), &s);
        }
        stream.push(s);
    }
    w.aggregate("стрим: 20 сбросов", &stream);
    w.step("commit хода", |_, ctx| {
        ctx.commit_streaming_tail();
        ctx.pending.set(false);
    });

    // (d) Прокрутка колесом: 8 шагов вверх, 4 обратно; инерцию тикает
    // `animate`, как цикл событий между кадрами.
    let y0 = w.scroll_y();
    let mut steps = Vec::new();
    let mut dispatch = Duration::ZERO;
    for i in 0..12 {
        let delta = if i < 8 { 150.0 } else { -150.0 };
        let s = w.sample(|h, _| {
            let started = Instant::now();
            h.send_event(&Event::MouseWheel {
                delta,
                delta_x: 0.0,
                position: Point::new(VIEW_W / 2.0, VIEW_H / 2.0),
            });
            dispatch += started.elapsed();
            h.animate(Duration::from_millis(16));
        });
        steps.push(s);
    }
    let y1 = w.scroll_y();
    w.aggregate("прокрутка: 12 шагов", &steps);
    if print {
        println!(
            "N={n:<3} прокрутка: смещение {y0:.0} → {y1:.0} px, обработка колеса ср {:.3} мс",
            ms(dispatch) / steps.len() as f64
        );
    }
    assert!((y1 - y0).abs() > 1.0, "колесо не прокрутило ленту: {y0} → {y1}");
}

/// Контексты приложения, как в `run_desktop`, но с `HOME` во временном
/// каталоге: конфиг грузится дефолтным (прогон воспроизводим), а автосейв
/// ленты пишет файлы чата туда же.
fn install_app(home: &std::path::Path) {
    std::env::set_var("HOME", home);
    syngui::signal::allow_signal_reads_on_this_thread();
    let (_theme, app) = crate::build_context();
    crate::i18n::install(app.general);
    provide_context(app);
    provide_context(SynChatCtx::new());
    provide_context(crate::syn_chat::SynModelRegistry::new());
    crate::install_syn_chat_autosave();
    syngui::signal::drain_and_run_effects();

    let ctx = use_context::<SynChatCtx>();
    ctx.chats.update(|list| {
        list.push(ChatMeta {
            id: CHAT_ID.to_string(),
            title: CHAT_TITLE.to_string(),
            preview: String::new(),
            created_at: 1,
            updated_at: 1,
            model_name: None,
            archived: false,
        })
    });
}

#[test]
fn synthetic_chat_is_deterministic_and_realistic() {
    let a = synthetic_chat(70, 7);
    let b = synthetic_chat(70, 7);
    assert_eq!(a.len(), 70);
    // Время у сообщений — от часов, сравниваем содержимое.
    let content = |v: &[ChatMsg]| -> Vec<(String, String)> {
        v.iter().map(|m| (m.body.clone(), m.thinking.clone())).collect()
    };
    assert_eq!(content(&a), content(&b), "генератор обязан давать один и тот же чат");
    let last = a.last().unwrap();
    assert!(last.role == ChatMsgRole::Assistant && matches!(last.kind, ChatMsgKind::Text));
    assert!(a.iter().any(|m| matches!(m.kind, ChatMsgKind::ToolCall { .. })));
    assert!(a.iter().any(|m| matches!(m.kind, ChatMsgKind::ToolResult { .. })));
    assert!(a.iter().any(|m| !m.thinking.is_empty()));
    assert!(a.iter().any(|m| m.body.contains("```")));
    let parts = chunks("абвгдеёжзи", 3);
    assert_eq!(parts.concat(), "абвгдеёжзи");
}

#[test]
#[ignore = "замер: печатает числа, меняет HOME процесса — только отдельным прогоном"]
fn feed_bench_report() {
    let home = tempfile::tempdir().expect("временный HOME");
    install_app(home.path());
    println!(
        "feed_bench: вид {VIEW_W}×{VIEW_H}, режим tool-карточек «{}», текст — Proportional (без атласа шрифтов), сборка {}",
        use_context::<AppCtx>().general.tool_display_mode.get_untracked(),
        if cfg!(debug_assertions) { "debug" } else { "release" },
    );
    // Прогрев: ленивые синтаксисы syntect, каталоги i18n и прочие
    // однократные расходы не попадают в числа замера.
    run(12, false);
    let slots_before = counters::snapshot().signal_slots;
    run(70, true);
    run(300, true);
    println!(
        "\nслотов сигналов: {} (+{} за замеры 70 и 300; слоты не освобождаются)",
        counters::snapshot().signal_slots,
        counters::snapshot().signal_slots - slots_before
    );
}
