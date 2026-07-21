//! Парсер Server-Sent Events поверх `reqwest::Response::bytes_stream()`.
//!
//! Реализован как `Stream` с ручным `poll_next`, без вспомогательных
//! крейтов вроде `async-stream`. Поддерживает:
//!
//! * инкрементальные байтовые чанки (одно событие может прийти несколькими
//!   частями TCP, а один чанк может содержать несколько событий);
//!
//! * разделители `\n\n` и `\r\n\r\n`;
//!
//! * многострочные `data:` (склеиваются `\n`) и `event: <name>`;
//!
//! * специальные маркеры OpenAI `data: [DONE]` — такое событие возвращается
//!   как обычное, с `data == "[DONE]"`; клиент решает, когда остановиться
//!   (обычно — при увиденном `[DONE]`).

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::stream::Stream;

use super::error::LlamaError;

/// Одно SSE-событие.
#[derive(Debug, Clone, Default)]
pub struct SseEvent {
    /// Значение `event:` (без этого поля в сыром протоколе — `None`).
    pub event: Option<String>,
    /// Склеенные строки `data:` (через `\n`).
    pub data: String,
    /// Значение `id:` (редко используется серверами llama.cpp).
    pub id: Option<String>,
    /// Значение `retry:` (мс), если присутствовало.
    pub retry_ms: Option<u64>,
}

impl SseEvent {
    /// Это маркер конца OpenAI-стрима `[DONE]`?
    pub fn is_done(&self) -> bool {
        self.data == "[DONE]"
    }

    /// Это серверный `event: error`?
    pub fn is_error(&self) -> bool {
        self.event.as_deref() == Some("error")
    }

    /// Попытаться распарсить `data` как JSON.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, LlamaError> {
        serde_json::from_str::<T>(&self.data).map_err(|e| LlamaError::Decode {
            source: e,
            body_hint: self
                .data
                .chars()
                .take(512)
                .collect::<String>(),
        })
    }
}

type InnerStream = Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>;

/// Стрим SSE-событий. `poll_next` возвращает `Result<SseEvent, LlamaError>`.
pub struct SseStream {
    inner: InnerStream,
    buf: Vec<u8>,
    /// Помечен `true`, если внутренний стрим уже завершился — тогда мы
    /// отдаём остаток `buf` как последнее событие (если оно непустое).
    eof: bool,
}

impl SseStream {
    /// Создать из reqwest::Response. Тело читается как bytes_stream.
    pub fn from_response(resp: reqwest::Response) -> Self {
        let inner: InnerStream = Box::pin(resp.bytes_stream());
        Self {
            inner,
            buf: Vec::with_capacity(2048),
            eof: false,
        }
    }

    /// Принять готовое значение для тестов (byte-сегменты).
    #[cfg(test)]
    pub fn from_byte_chunks(chunks: Vec<Bytes>) -> Self {
        use futures_util::stream;
        let s: InnerStream = Box::pin(stream::iter(chunks.into_iter().map(Ok)));
        Self {
            inner: s,
            buf: Vec::new(),
            eof: false,
        }
    }
}

impl Stream for SseStream {
    type Item = Result<SseEvent, LlamaError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // 1) Пробуем вытащить готовое событие из буфера.
            if let Some((event_raw, consumed)) = take_event(&self.buf) {
                // Сдвигаем буфер (drain).
                self.buf.drain(..consumed);
                if !event_raw.is_empty() {
                    return Poll::Ready(Some(Ok(parse_event(&event_raw))));
                }
                // Пустое событие — игнорируем, попробуем ещё.
                continue;
            }

            if self.eof {
                // Остаток без `\n\n` на конце — последнее событие, если в нём есть контент.
                if !self.buf.is_empty() {
                    let raw = std::mem::take(&mut self.buf);
                    let event = parse_event(&raw);
                    if event.data.is_empty() && event.event.is_none() {
                        return Poll::Ready(None);
                    }
                    return Poll::Ready(Some(Ok(event)));
                }
                return Poll::Ready(None);
            }

            // 2) Читаем следующий чанк.
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    self.eof = true;
                    // Второй проход: вернём остаток.
                    continue;
                }
                Poll::Ready(Some(Ok(chunk))) => {
                    self.buf.extend_from_slice(&chunk);
                    continue;
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(LlamaError::from(e))));
                }
            }
        }
    }
}

/// Ищем конец первого события (`\n\n` или `\r\n\r\n`). Возвращает
/// `(байты события без разделителя, сколько байт потреблено вместе с разделителем)`.
fn take_event(buf: &[u8]) -> Option<(Vec<u8>, usize)> {
    let len = buf.len();
    let mut i = 0;
    while i + 1 < len {
        // \n\n
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some((buf[..i].to_vec(), i + 2));
        }
        // \r\n\r\n
        if i + 3 < len
            && buf[i] == b'\r'
            && buf[i + 1] == b'\n'
            && buf[i + 2] == b'\r'
            && buf[i + 3] == b'\n'
        {
            return Some((buf[..i].to_vec(), i + 4));
        }
        i += 1;
    }
    None
}

/// Распарсить сырое событие (байты без завершающего `\n\n`).
fn parse_event(raw: &[u8]) -> SseEvent {
    let mut event = SseEvent::default();
    let mut data_parts: Vec<String> = Vec::new();

    for line in split_lines(raw) {
        // Комментарий (строка начинается с `:`) — пропускаем.
        if line.is_empty() || line.starts_with(b":") {
            continue;
        }
        let (field, value) = match split_field(line) {
            Some(pair) => pair,
            None => continue,
        };
        let value_str = std::str::from_utf8(value).unwrap_or("").to_string();
        match field {
            b"event" => event.event = Some(value_str),
            b"data" => data_parts.push(value_str),
            b"id" => event.id = Some(value_str),
            b"retry" => {
                if let Ok(ms) = value_str.trim().parse::<u64>() {
                    event.retry_ms = Some(ms);
                }
            }
            _ => {}
        }
    }
    event.data = data_parts.join("\n");
    event
}

/// Разбивает байты на строки по `\n` / `\r\n` / `\r`.
fn split_lines(buf: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut lines: Vec<&[u8]> = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < buf.len() {
        let c = buf[i];
        if c == b'\n' {
            lines.push(&buf[start..i]);
            start = i + 1;
        } else if c == b'\r' {
            lines.push(&buf[start..i]);
            if i + 1 < buf.len() && buf[i + 1] == b'\n' {
                start = i + 2;
                i += 1;
            } else {
                start = i + 1;
            }
        }
        i += 1;
    }
    if start < buf.len() {
        lines.push(&buf[start..]);
    }
    lines.into_iter()
}

/// Разбивает строку `field: value` по первому двоеточию. Допускаем и
/// `field:value` без пробела (по спецификации пробел после `:` — опционален).
fn split_field(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let pos = line.iter().position(|b| *b == b':')?;
    let field = &line[..pos];
    let mut value = &line[pos + 1..];
    if value.first() == Some(&b' ') {
        value = &value[1..];
    }
    Some((field, value))
}

// ───────────────────────────────────────────────────────────────────────────
// Тесты
// ───────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures_util::StreamExt;

    fn chunk(s: &str) -> Bytes {
        Bytes::copy_from_slice(s.as_bytes())
    }

    #[tokio::test]
    async fn parses_two_events_in_one_chunk() {
        let s = SseStream::from_byte_chunks(vec![chunk(
            "event: data\ndata: {\"a\":1}\n\nevent: data\ndata: {\"a\":2}\n\n",
        )]);
        let events: Vec<_> = s.collect().await;
        assert_eq!(events.len(), 2);
        let e1 = events[0].as_ref().unwrap();
        assert_eq!(e1.event.as_deref(), Some("data"));
        assert_eq!(e1.data, "{\"a\":1}");
    }

    #[tokio::test]
    async fn handles_byte_split_chunks() {
        let raw = "data: hello\n\ndata: world\n\n";
        // Разбиваем по одному байту, чтобы проверить ресайклинг буфера.
        let chunks: Vec<Bytes> = raw
            .as_bytes()
            .iter()
            .map(|b| Bytes::copy_from_slice(&[*b]))
            .collect();
        let s = SseStream::from_byte_chunks(chunks);
        let events: Vec<_> = s.collect().await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].as_ref().unwrap().data, "hello");
        assert_eq!(events[1].as_ref().unwrap().data, "world");
    }

    #[tokio::test]
    async fn recognizes_done_marker() {
        let s = SseStream::from_byte_chunks(vec![chunk("data: [DONE]\n\n")]);
        let events: Vec<_> = s.collect().await;
        assert_eq!(events.len(), 1);
        let e = events[0].as_ref().unwrap();
        assert!(e.is_done());
    }

    #[tokio::test]
    async fn handles_event_error() {
        let s = SseStream::from_byte_chunks(vec![chunk(
            "event: error\ndata: {\"error\":{\"code\":500,\"message\":\"oops\",\"type\":\"server_error\"}}\n\n",
        )]);
        let events: Vec<_> = s.collect().await;
        let e = events[0].as_ref().unwrap();
        assert!(e.is_error());
        assert!(e.data.contains("oops"));
    }

    #[tokio::test]
    async fn multiline_data_concatenates_with_newline() {
        let s = SseStream::from_byte_chunks(vec![chunk("data: line1\ndata: line2\n\n")]);
        let events: Vec<_> = s.collect().await;
        assert_eq!(events[0].as_ref().unwrap().data, "line1\nline2");
    }

    #[tokio::test]
    async fn trailing_event_without_blank_line_is_emitted() {
        let s = SseStream::from_byte_chunks(vec![chunk("data: tail")]);
        let events: Vec<_> = s.collect().await;
        assert_eq!(events[0].as_ref().unwrap().data, "tail");
    }

    #[tokio::test]
    async fn crlf_terminators_are_supported() {
        let s = SseStream::from_byte_chunks(vec![chunk("data: x\r\n\r\ndata: y\r\n\r\n")]);
        let events: Vec<_> = s.collect().await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].as_ref().unwrap().data, "x");
        assert_eq!(events[1].as_ref().unwrap().data, "y");
    }

    #[tokio::test]
    async fn comment_lines_are_ignored() {
        let s = SseStream::from_byte_chunks(vec![chunk(": heartbeat\ndata: live\n\n")]);
        let events: Vec<_> = s.collect().await;
        assert_eq!(events[0].as_ref().unwrap().data, "live");
    }
}
