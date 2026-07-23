//! Стейт-машина выделения reasoning-блоков `<think>...</think>` из стрим-content.
//!
//! Когда llama-server запущен с `reasoning_format=none` (или без поддержки
//! reasoning_content вообще), модель эмиттит chain-of-thought прямо в
//! `delta.content`, обернутую тегами:
//!
//! ```text
//! <think>
//! пользователь спрашивает X, мне нужно прикинуть Y, …
//! </think>
//! Вот ответ: …
//! ```
//!
//! Парсер инкрементальный: чанки приходят произвольно нарезанными по
//! байтам/символам, открывающий или закрывающий тег может разорваться
//! по чанкам. Состояние держится между вызовами [`ThinkParser::feed`];
//! на выходе — две независимые «протёкшие» части: `body` (текст ответа)
//! и `thinking` (содержимое внутри тегов). Тег сам в выход не попадает.
//!
//! Не парсим вложенные `<think>` (модели так не пишут): второй открывающий
//! тег внутри блока остаётся как литеральный текст thinking — это
//! безопасный fallback.
//!
//! Два режима старта:
//! - [`ThinkParser::new`] — `inside=false`. Парсер ждёт буквального `<think>`
//!   в потоке. Подходит llama-server-у и моделям, которые сами выписывают
//!   open-тег.
//! - [`ThinkParser::new_implicit_open`] — `inside=true` + проглатывание
//!   повторных `</think>` после закрытия. Нужен когда chat-template
//!   подаёт `<think>\n` прямо в prompt (Qwen3-VL / Qwen3-Thinking 2507) и
//!   модель никогда не пишет открывающий тег сама — только закрывающий.

/// Открывающий тег `<think>`. Сравнение case-insensitive (DeepSeek
/// использует lower-case, но перестрахуемся от вариаций).
const OPEN: &str = "<think>";
/// Закрывающий тег `</think>`. Длина 8 — это верхняя граница для
/// `pending`-буфера незавершённого совпадения.
const CLOSE: &str = "</think>";

/// Инкрементальный парсер: поддерживает текущее состояние (внутри/снаружи
/// блока) и буфер незавершённого префикса тега, разорванного между чанками.
#[derive(Default, Debug)]
pub struct ThinkParser {
    /// `true` — текущий курсор находится внутри `<think>...</think>`.
    inside: bool,
    /// Хвост предыдущего чанка, который потенциально может быть началом
    /// тега. Не более `CLOSE.len() - 1 = 7` символов.
    pending: String,
    /// `true` — после закрытия `</think>` повторные `</think>` (с любым
    /// whitespace между ними) проглатываются как noise, не попадая в
    /// `body`. Включается [`Self::new_implicit_open`]: наблюдалось, что
    /// Qwen3-VL/Thinking иногда выписывает `</think>\n</think>` на пустом
    /// thinking-step.
    eat_extraneous_close: bool,
}

/// Результат разбора одного чанка: что отдать в `ChatMsg.body` и что —
/// в `ChatMsg.thinking`. Любая из частей может быть пустой.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ThinkSplit {
    pub body: String,
    pub thinking: String,
}

impl ThinkParser {
    /// Парсер в начальном состоянии (вне блока, пустой буфер). Ждёт
    /// буквального `<think>` в потоке.
    pub fn new() -> Self {
        Self::default()
    }

    /// Парсер для случая, когда chat-template уже подал `<think>` прямо в
    /// prompt — модель не пишет открывающий тег, только закрывающий.
    /// Стартует `inside=true`. Дополнительно проглатывает повторные
    /// `</think>` после первого закрытия (артефакт Qwen3-VL: пустой
    /// thinking-step иногда закрывается дважды).
    pub fn new_implicit_open() -> Self {
        Self {
            inside: true,
            pending: String::new(),
            eat_extraneous_close: true,
        }
    }

    /// Скормить очередной кусок content и получить разнесённый по
    /// `body` / `thinking` результат. Парсер обновляет внутреннее
    /// состояние; вызывайте подряд для всех чанков одного assistant-turn'а.
    pub fn feed(&mut self, chunk: &str) -> ThinkSplit {
        let mut buf = std::mem::take(&mut self.pending);
        buf.push_str(chunk);

        let mut out = ThinkSplit::default();
        let mut cursor = 0usize;

        loop {
            if self.inside {
                // Ищем закрывающий `</think>`.
                match find_ci(&buf, CLOSE, cursor) {
                    Some(pos) => {
                        push_segment(&mut out, true, &buf[cursor..pos]);
                        cursor = pos + CLOSE.len();
                        self.inside = false;
                    }
                    None => {
                        // Закрывающий тег не пришёл целиком — придерживаем
                        // потенциальный префикс на стыке чанков.
                        let split = trailing_tag_prefix(&buf[cursor..], CLOSE);
                        let safe_end = cursor + split;
                        push_segment(&mut out, true, &buf[cursor..safe_end]);
                        self.pending = buf[safe_end..].to_string();
                        return out;
                    }
                }
            } else {
                // Снаружи. Всегда ищем OPEN; если `eat_extraneous_close=true`
                // — также ищем CLOSE и берём тот тег, что встретился раньше.
                // При совпадении выбираем OPEN (открытие важнее).
                let open_pos = find_ci(&buf, OPEN, cursor);
                let close_pos = if self.eat_extraneous_close {
                    find_ci(&buf, CLOSE, cursor)
                } else {
                    None
                };

                let (next_pos, tag_len, opens) = match (open_pos, close_pos) {
                    (None, None) => {
                        let split_open = trailing_tag_prefix(&buf[cursor..], OPEN);
                        let split = if self.eat_extraneous_close {
                            let split_close = trailing_tag_prefix(&buf[cursor..], CLOSE);
                            split_open.min(split_close)
                        } else {
                            split_open
                        };
                        let safe_end = cursor + split;
                        push_segment(&mut out, false, &buf[cursor..safe_end]);
                        self.pending = buf[safe_end..].to_string();
                        return out;
                    }
                    (Some(o), None) => (o, OPEN.len(), true),
                    (None, Some(c)) => (c, CLOSE.len(), false),
                    (Some(o), Some(c)) => {
                        if o <= c {
                            (o, OPEN.len(), true)
                        } else {
                            (c, CLOSE.len(), false)
                        }
                    }
                };

                push_segment(&mut out, false, &buf[cursor..next_pos]);
                cursor = next_pos + tag_len;
                if opens {
                    self.inside = true;
                }
                // else: extraneous CLOSE проглочен, остаёмся снаружи.
            }
        }
    }
}

/// Безопасно довешивает `segment` к нужной секции, не плодя пустых
/// аллокаций.
fn push_segment(out: &mut ThinkSplit, inside: bool, segment: &str) {
    if segment.is_empty() {
        return;
    }
    if inside {
        out.thinking.push_str(segment);
    } else {
        out.body.push_str(segment);
    }
}

/// Возвращает байтовое смещение первого совпадения `needle` в `haystack`
/// начиная с `from`, без учёта регистра. Только ASCII-сравнение —
/// теги `<think>` и `</think>` чисто ASCII, поэтому достаточно.
fn find_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from);
    }
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if from + n.len() > h.len() {
        return None;
    }
    let mut i = from;
    while i + n.len() <= h.len() {
        if h[i..i + n.len()]
            .iter()
            .zip(n.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Сколько байт с **конца** `s` могут быть началом потенциального `tag`.
/// Возвращает индекс в `s`, до которого можно безопасно отдать содержимое
/// в out-секцию; всё после — лежит в `pending`.
///
/// Пример: `s = "abc<thi"`, `tag = "<think>"` → safe_end = 3 (отдаём
/// `"abc"`, в pending кладём `"<thi"`).
fn trailing_tag_prefix(s: &str, tag: &str) -> usize {
    let tag_bytes = tag.as_bytes();
    let s_bytes = s.as_bytes();
    let max_check = tag_bytes.len().saturating_sub(1).min(s_bytes.len());

    // Пробуем максимально длинный потенциальный префикс. `i` — длина
    // префикса тега в конце строки.
    for i in (1..=max_check).rev() {
        let tail = &s_bytes[s_bytes.len() - i..];
        // Сначала проверка char-boundary, чтобы не разорвать UTF-8.
        let cut = s_bytes.len() - i;
        if !s.is_char_boundary(cut) {
            continue;
        }
        if tail
            .iter()
            .zip(tag_bytes.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return cut;
        }
    }
    s.len()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn split(parser: &mut ThinkParser, chunks: &[&str]) -> ThinkSplit {
        let mut acc = ThinkSplit::default();
        for c in chunks {
            let part = parser.feed(c);
            acc.body.push_str(&part.body);
            acc.thinking.push_str(&part.thinking);
        }
        acc
    }

    #[test]
    fn whole_block_in_one_chunk() {
        let mut p = ThinkParser::new();
        let r = split(&mut p, &["<think>a</think>b"]);
        assert_eq!(r.body, "b");
        assert_eq!(r.thinking, "a");
    }

    #[test]
    fn body_only_no_tags() {
        let mut p = ThinkParser::new();
        let r = split(&mut p, &["hello world"]);
        assert_eq!(r.body, "hello world");
        assert_eq!(r.thinking, "");
    }

    #[test]
    fn thinking_only_complete() {
        let mut p = ThinkParser::new();
        let r = split(&mut p, &["<think>x</think>"]);
        assert_eq!(r.body, "");
        assert_eq!(r.thinking, "x");
    }

    #[test]
    fn open_tag_split_across_chunks() {
        let mut p = ThinkParser::new();
        let r = split(&mut p, &["<thi", "nk>x</thi", "nk>y"]);
        assert_eq!(r.body, "y");
        assert_eq!(r.thinking, "x");
    }

    #[test]
    fn close_tag_split_at_last_byte() {
        let mut p = ThinkParser::new();
        // `</think` пришло без `>` — нужно ждать, не отдавать как thinking.
        let r = split(&mut p, &["<think>abc</think", ">tail"]);
        assert_eq!(r.body, "tail");
        assert_eq!(r.thinking, "abc");
    }

    #[test]
    fn open_without_close_streams_thinking_so_far() {
        let mut p = ThinkParser::new();
        // Поток ещё не завершён — закрывающий тег не пришёл. Всё внутри
        // должно течь в thinking, body — пустой.
        let r = split(&mut p, &["<think>abc"]);
        assert_eq!(r.body, "");
        assert_eq!(r.thinking, "abc");
    }

    #[test]
    fn case_insensitive_tags() {
        let mut p = ThinkParser::new();
        let r = split(&mut p, &["<THINK>X</Think>Y"]);
        assert_eq!(r.body, "Y");
        assert_eq!(r.thinking, "X");
    }

    #[test]
    fn body_then_thinking_then_body() {
        let mut p = ThinkParser::new();
        let r = split(&mut p, &["pre<think>mid</think>post"]);
        assert_eq!(r.body, "prepost");
        assert_eq!(r.thinking, "mid");
    }

    #[test]
    fn pending_does_not_eat_short_lookalike() {
        let mut p = ThinkParser::new();
        // `<` в конце чанка — потенциальное начало тега; следующий чанк
        // начинается с НЕ `t` — должны корректно отдать всё в body.
        let r = split(&mut p, &["abc<", "def"]);
        assert_eq!(r.body, "abc<def");
        assert_eq!(r.thinking, "");
    }

    #[test]
    fn utf8_boundary_safe() {
        let mut p = ThinkParser::new();
        // Кириллица + потенциальный начальный байт тега в конце.
        let r = split(&mut p, &["приве", "т<think>да</think>!"]);
        assert_eq!(r.body, "привет!");
        assert_eq!(r.thinking, "да");
    }

    // ── implicit-open режим (chat-template уже подал `<think>` в prompt) ──

    #[test]
    fn implicit_open_thinking_then_close_then_body() {
        let mut p = ThinkParser::new_implicit_open();
        // Модель: размышляет, закрывает, выдаёт ответ.
        let r = split(&mut p, &["рассужден", "ия про X</think>финальный ответ"]);
        assert_eq!(r.thinking, "рассуждения про X");
        assert_eq!(r.body, "финальный ответ");
    }

    #[test]
    fn implicit_open_double_close_eaten() {
        let mut p = ThinkParser::new_implicit_open();
        // Реальный кейс из логов Qwen3-VL: пустой/короткий thinking
        // закрылся дважды, между тегами whitespace.
        let r = split(&mut p, &["abc</think>\n</think>tail"]);
        assert_eq!(r.thinking, "abc");
        assert_eq!(r.body, "\ntail");
    }

    #[test]
    fn implicit_open_no_close_streams_thinking_only() {
        let mut p = ThinkParser::new_implicit_open();
        // Поток оборвался внутри thinking — body пустой, thinking всё.
        let r = split(&mut p, &["размышляю про", " что-то"]);
        assert_eq!(r.thinking, "размышляю про что-то");
        assert_eq!(r.body, "");
    }

    #[test]
    fn implicit_open_explicit_reopen_after_close() {
        let mut p = ThinkParser::new_implicit_open();
        // Маловероятный кейс: модель сама открыла второй think-блок после
        // ответа. OPEN ранее CLOSE → открываем, не съедаем.
        let r = split(&mut p, &["mid</think>body<think>more</think>tail"]);
        assert_eq!(r.thinking, "midmore");
        assert_eq!(r.body, "bodytail");
    }

    #[test]
    fn implicit_open_split_close_across_chunks() {
        let mut p = ThinkParser::new_implicit_open();
        let r = split(&mut p, &["abc</thi", "nk>tail"]);
        assert_eq!(r.thinking, "abc");
        assert_eq!(r.body, "tail");
    }

    #[test]
    fn implicit_open_extraneous_close_in_separate_feed() {
        let mut p = ThinkParser::new_implicit_open();
        // Сначала закрытие пришло в одном чанке, потом второй `</think>`
        // отдельным чанком — должен быть проглочен.
        let r1 = p.feed("abc</think>");
        assert_eq!(r1.thinking, "abc");
        assert_eq!(r1.body, "");
        let r2 = p.feed("</think>tail");
        assert_eq!(r2.thinking, "");
        assert_eq!(r2.body, "tail");
    }
}
