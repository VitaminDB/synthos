//! Кэш vision-эмбеддингов вложений.
//!
//! Промпт пересобирается на каждом turn'е agent-loop и на каждой
//! регенерации, а картинки и видео из истории чата в нём остаются. Без
//! кэша каждая итерация заново гоняла бы vision-башню по всем вложениям
//! ленты — секунды GPU-времени на ровном месте.
//!
//! Ключ — sha256 содержимого + потолок токенов, поэтому кэш валиден и при
//! повторном прикреплении того же файла в другом чате. Тензоры лежат на
//! устройстве модели, так что объём ограничен бюджетом в токенах
//! ([`TOKEN_BUDGET`]); вытеснение — FIFO.
//!
//! Кэш обнуляется при выгрузке/смене модели ([`clear`]): эмбеддинги
//! привязаны к конкретной vision-башне и к её VRAM.

use std::sync::Mutex;

use synaptix::facade::llm::{MediaEmbedding, MediaKind};

/// Потолок «горячих» вложений в токенах. 24576 ≈ 24 картинки по 1024
/// токена; при hidden 6144 и f16 это ~300 МБ VRAM.
const TOKEN_BUDGET: usize = 24_576;

struct Entry {
    key: String,
    embedding: MediaEmbedding,
}

static CACHE: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

fn key_of(sha: &str, kind: MediaKind, max_tokens: Option<usize>) -> String {
    let k = match kind {
        MediaKind::Image => "img",
        MediaKind::Video => "vid",
    };
    format!("{sha}:{k}:{}", max_tokens.unwrap_or(0))
}

/// Есть ли эмбеддинг в кэше. Дешевле [`get`]: не клонирует тензор.
pub fn has(sha: &str, kind: MediaKind, max_tokens: Option<usize>) -> bool {
    let key = key_of(sha, kind, max_tokens);
    CACHE
        .lock()
        .map(|g| g.iter().any(|e| e.key == key))
        .unwrap_or(false)
}

/// Достаёт эмбеддинг из кэша, если он там есть.
pub fn get(sha: &str, kind: MediaKind, max_tokens: Option<usize>) -> Option<MediaEmbedding> {
    let key = key_of(sha, kind, max_tokens);
    let guard = CACHE.lock().ok()?;
    guard
        .iter()
        .find(|e| e.key == key)
        .map(|e| e.embedding.clone())
}

/// Кладёт эмбеддинг в кэш, вытесняя самые старые записи сверх бюджета.
pub fn put(sha: &str, kind: MediaKind, max_tokens: Option<usize>, embedding: &MediaEmbedding) {
    let key = key_of(sha, kind, max_tokens);
    let Ok(mut guard) = CACHE.lock() else {
        return;
    };
    if guard.iter().any(|e| e.key == key) {
        return;
    }
    guard.push(Entry { key, embedding: embedding.clone() });
    let mut total: usize = guard.iter().map(|e| e.embedding.tokens).sum();
    while total > TOKEN_BUDGET && guard.len() > 1 {
        let dropped = guard.remove(0);
        total -= dropped.embedding.tokens;
    }
}

/// Полностью сбрасывает кэш — при выгрузке или смене модели.
pub fn clear() {
    if let Ok(mut guard) = CACHE.lock() {
        if !guard.is_empty() {
            log::info!("[attach] сброс кэша vision-эмбеддингов: {} записей", guard.len());
            guard.clear();
        }
    }
}
