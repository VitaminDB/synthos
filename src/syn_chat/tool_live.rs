//! Живой вывод идущего вызова `bash` — только для глаз пользователя.
//!
//! Модель получает результат целиком и как раньше (`executor::execute`):
//! живой хвост не попадает ни в историю, ни в ленту, лишних токенов нет.
//! Здесь — разбор потока в строки так, как их показал бы терминал: `\r`
//! перерисовывает текущую строку (прогресс-бары), ANSI-последовательности
//! вырезаются, держится только хвост из [`KEEP_LINES`] строк.

/// Сколько последних строк держать: карточка показывает меньше, запас —
/// на случай, если хвост вырастет до конца строки после `\r`.
pub const KEEP_LINES: usize = 64;
/// Предел длины одной строки: бинарный мусор без `\n` не раздует UI.
const MAX_LINE_CHARS: usize = 400;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveLine {
    pub text: String,
    /// Строка пришла из stderr.
    pub err: bool,
}

/// Хвост живого вывода одного вызова.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveOutput {
    /// Чат хода — чтобы не показывать вывод в чужом открытом чате.
    pub owner: Option<String>,
    /// `tool_call_id` вызова — карточка, к которой относится вывод.
    pub call_id: String,
    /// Завершённые строки (хвост).
    pub lines: Vec<LiveLine>,
    /// Недописанные строки stdout/stderr (без `\n`) — показываются последними.
    open: [Option<LiveLine>; 2],
    /// Всего завершённых строк за вызов, включая вытесненные из хвоста.
    pub total: usize,
}

impl LiveOutput {
    pub fn new(owner: Option<String>, call_id: String) -> Self {
        Self { owner, call_id, ..Self::default() }
    }

    pub fn push(&mut self, err: bool, chunk: &str) {
        let slot = err as usize;
        let clean = strip_ansi(chunk);
        let mut rest = clean.as_str();
        while !rest.is_empty() {
            let cut = rest.find(['\n', '\r']).unwrap_or(rest.len());
            let (piece, tail) = rest.split_at(cut);
            let line = self.open[slot].get_or_insert_with(|| LiveLine { text: String::new(), err });
            if line.text.chars().count() < MAX_LINE_CHARS {
                line.text.push_str(piece);
            }
            match tail.as_bytes().first() {
                Some(b'\n') => {
                    let done = self.open[slot].take().unwrap_or_default();
                    self.lines.push(done);
                    self.total += 1;
                    if self.lines.len() > KEEP_LINES {
                        self.lines.drain(..self.lines.len() - KEEP_LINES);
                    }
                    rest = &tail[1..];
                }
                // `\r\n` — обычный перевод строки; одиночный `\r` —
                // перерисовка текущей строки.
                Some(b'\r') if tail.as_bytes().get(1) == Some(&b'\n') => rest = &tail[1..],
                Some(b'\r') => {
                    if let Some(l) = self.open[slot].as_mut() {
                        l.text.clear();
                    }
                    rest = &tail[1..];
                }
                _ => rest = tail,
            }
        }
    }

    /// Последние `n` строк для показа, включая недописанные.
    pub fn tail(&self, n: usize) -> Vec<LiveLine> {
        let open = self.open.iter().flatten().filter(|l| !l.text.is_empty());
        let all: Vec<LiveLine> = self.lines.iter().chain(open).cloned().collect();
        all[all.len().saturating_sub(n)..].to_vec()
    }

    /// Строк всего, с недописанными.
    pub fn line_count(&self) -> usize {
        self.total + self.open.iter().flatten().filter(|l| !l.text.is_empty()).count()
    }
}

/// Вырезать ANSI escape-последовательности (цвета, движение курсора).
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            if c == '\t' || c == '\n' || c == '\r' || !c.is_control() {
                out.push(c);
            }
            continue;
        }
        match it.next() {
            // CSI: ESC [ параметры… финальный байт @–~
            Some('[') => {
                for c in it.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: ESC ] … BEL | ESC \
            Some(']') => {
                while let Some(c) = it.next() {
                    if c == '\x07' || (c == '\x1b' && it.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(o: &LiveOutput) -> Vec<String> {
        o.tail(100).into_iter().map(|l| l.text).collect()
    }

    #[test]
    fn lines_split_across_chunks() {
        let mut o = LiveOutput::default();
        o.push(false, "wait");
        o.push(false, "ing…\nconn");
        assert_eq!(texts(&o), ["waiting…", "conn"]);
        o.push(false, "ected\n");
        assert_eq!(texts(&o), ["waiting…", "connected"]);
        assert_eq!(o.line_count(), 2);
    }

    #[test]
    fn carriage_return_redraws_line() {
        let mut o = LiveOutput::default();
        o.push(false, " 10%\r 50%\r100%\r\ndone\r\n");
        assert_eq!(texts(&o), ["100%", "done"]);
    }

    #[test]
    fn ansi_is_stripped_and_stderr_marked() {
        let mut o = LiveOutput::default();
        o.push(true, "\x1b[1;31merror\x1b[0m: no device\n");
        o.push(false, "\x1b]0;title\x07ok\n");
        let t = o.tail(10);
        assert_eq!((t[0].text.as_str(), t[0].err), ("error: no device", true));
        assert_eq!((t[1].text.as_str(), t[1].err), ("ok", false));
    }

    #[test]
    fn tail_is_bounded() {
        let mut o = LiveOutput::default();
        for i in 0..200 {
            o.push(false, &format!("{i}\n"));
        }
        assert_eq!(o.lines.len(), KEEP_LINES);
        assert_eq!(o.total, 200);
        assert_eq!(texts(&o).last().unwrap(), "199");
    }
}
