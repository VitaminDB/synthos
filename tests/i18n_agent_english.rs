//! Тексты, уходящие модели (система, описания инструментов, tool-результаты,
//! промпты компактификации) — на английском и не переключаются с языком
//! интерфейса (фаза 9 плана i18n, см. `docs/i18n_2026.md`). Тест ловит
//! регресс: случайно вернувшуюся кириллицу в этих текстах.
//!
//! Область: `src/agent/**/*.rs`, `src/syn_chat/{system_prompt,compact,
//! channel_parser,tool_parser}.rs` — без комментариев (`//`, `/* */`) и без
//! тестовых модулей (`#[cfg(test)]` и всё, что после него, а также файлы
//! `tests.rs` / `harness_tests.rs` целиком, тест не смотрит:
//! там кириллица — валидные фикстуры для парсеров, а не текст для модели);
//! плюс `DEFAULT_SYSTEM_PROMPT`/`DEFAULT_VOICE_REFINE_PROMPT` из
//! `src/config.rs` — читаются напрямую из `synthos::config`, а не файловым
//! сканом (у файла `config.rs` есть и другие, UI-шные константы, которые
//! сканить не нужно).
//!
//! Точечные исключения (кириллица вне тестов, но вне области фазы 9) —
//! в [`ALLOWED`], с причиной для каждой строки.

use std::path::{Path, PathBuf};

use synthos::config::{DEFAULT_SYSTEM_PROMPT, DEFAULT_VOICE_REFINE_PROMPT};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn agent_rs_files(dir: &Path, acc: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            agent_rs_files(&p, acc);
        } else if p.extension().is_some_and(|x| x == "rs") {
            // Файл-модуль тестов (`mod tests;` рядом с кодом) целиком
            // тестовый: фикстуры в нём русские по делу — названия
            // страниц, колонок и карточек проверяются через `tr!`.
            if p.file_name().is_some_and(|n| n == "tests.rs" || n == "harness_tests.rs") {
                continue;
            }
            acc.push(p);
        }
    }
}

/// `true`, если в строке (после вычитки комментариев/диагностики/тестов)
/// есть кириллица.
fn has_cyrillic(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '\u{0400}'..='\u{04FF}'))
}

/// Обрубает файл на первом `#[cfg(test)]` — во всех проверяемых файлах
/// тестовый модуль идёт последним блоком, так что дальше только фикстуры.
fn drop_test_module(src: &str) -> &str {
    src.find("#[cfg(test)]").map(|idx| &src[..idx]).unwrap_or(src)
}

/// Длина UTF-8 символа по ведущему байту (для одного Unicode-скаляра внутри
/// символьного литерала `'X'`).
fn utf8_char_len(lead: u8) -> usize {
    if lead & 0x80 == 0 {
        1
    } else if lead & 0xE0 == 0xC0 {
        2
    } else if lead & 0xF0 == 0xE0 {
        3
    } else {
        4
    }
}

/// Если начиная с позиции `quote` (индекс открывающей `'`) стоит символьный
/// литерал (`'x'`, `'\n'`, `'\''`, `'\u{2022}'`) — возвращает индекс сразу
/// после закрывающей `'`. `None` — это не литерал, а лайфтайм (`'a`,
/// `'static`) или самостоятельный апостроф; лайфтайм никогда не закрывается
/// второй кавычкой, чем и отличается от литерала.
fn char_literal_end(b: &[u8], quote: usize) -> Option<usize> {
    let mut i = quote + 1;
    if i >= b.len() {
        return None;
    }
    if b[i] == b'\\' {
        i += 1;
        if i >= b.len() {
            return None;
        }
        if b[i] == b'u' && b.get(i + 1) == Some(&b'{') {
            i += 2;
            while i < b.len() && b[i] != b'}' {
                i += 1;
            }
            if i >= b.len() {
                return None;
            }
            i += 1; // за '}'
        } else {
            i += 1; // простой escape: \n, \t, \\, \', \" и т.п.
        }
    } else {
        i += utf8_char_len(b[i]);
    }
    if b.get(i) == Some(&b'\'') {
        Some(i + 1)
    } else {
        None
    }
}

/// Замена `//`- и `/* */`-комментариев на пробелы (переводы строк
/// сохраняются — номера строк в отчёте остаются настоящими). Учитывает
/// строковые литералы: `//` внутри `"..."` (например, в URL вида
/// `https://...`) — не начало комментария. `\"` внутри строки не закрывает
/// её. Символьные литералы (`'"'`, `'\\'`) распознаются отдельно — иначе
/// одинарная кавычка вокруг `"` сбивает счётчик строк и хвост файла
/// ошибочно считается «внутри строки». Raw-строки (`r#"…"#`) отдельно не
/// разбираются: единственное вхождение в проверяемой области
/// (CSS-селектор в `web/search.rs`) кириллицы не содержит, так что
/// огрубление здесь безопасно.
fn strip_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut in_string = false;
    let mut escape = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if in_string {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == b'\'' {
            if let Some(end) = char_literal_end(b, i) {
                i = end;
                continue;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_string = true;
            i += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                if b[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
            if i + 1 < b.len() {
                out[i] = b' ';
                out[i + 1] = b' ';
                i += 2;
            }
            continue;
        }
        i += 1;
    }
    String::from_utf8(out).expect("valid utf-8 preserved by byte-level blanking")
}

/// Триггеры, за которыми до балансной закрывающей скобки идёт диагностика —
/// не текст для модели и не для пользователя: `log::*!`/`tracing::*!` —
/// строки лога (ТЗ фазы 9 их не переводит), `eprintln!`/`println!` —
/// консольная диагностика, `panic!`/`unreachable!`/`unimplemented!`/`todo!`/
/// `.expect(` — сообщения нарушенных инвариантов, которые в норме никто не
/// видит. Убираются целиком, чтобы кириллица в аргументах (в т.ч. за
/// пределами первой строки макро-вызова) не считалась находкой.
const DIAGNOSTIC_TRIGGERS: &[&str] = &[
    "log::info!(",
    "log::warn!(",
    "log::debug!(",
    "log::error!(",
    "log::trace!(",
    "tracing::info!(",
    "tracing::warn!(",
    "tracing::debug!(",
    "tracing::error!(",
    "tracing::trace!(",
    "eprintln!(",
    "println!(",
    "panic!(",
    "unreachable!(",
    "unimplemented!(",
    "todo!(",
    ".expect(",
];

/// Индекс закрывающей `)`, соответствующей `(` в позиции `open`, с учётом
/// строковых и символьных литералов (скобки внутри `"..."`/`'x'` не
/// считаются — иначе, например, `')'`-литерал в аргументах сбил бы баланс).
fn find_matching_paren(b: &[u8], open: usize) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut i = open;
    while i < b.len() {
        let c = b[i];
        if in_string {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == b'\'' {
            if let Some(end) = char_literal_end(b, i) {
                i = end;
                continue;
            }
        }
        match c {
            b'"' => in_string = true,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn strip_diagnostic_calls(src: &str) -> String {
    let mut out = src.as_bytes().to_vec();
    let mut cursor = 0usize;
    loop {
        let hay = std::str::from_utf8(&out[cursor..]).unwrap_or("");
        let Some((rel, trigger)) = DIAGNOSTIC_TRIGGERS
            .iter()
            .filter_map(|t| hay.find(t).map(|p| (p, *t)))
            .min_by_key(|(p, _)| *p)
        else {
            break;
        };
        let start = cursor + rel;
        let open = start + trigger.len() - 1; // позиция '('
        let Some(close) = find_matching_paren(&out, open) else {
            // Незакрытая скобка быть не должна в валидном Rust — просто
            // прекращаем поиск, чтобы не зациклиться.
            break;
        };
        for b in out.iter_mut().take(close + 1).skip(start) {
            if *b != b'\n' {
                *b = b' ';
            }
        }
        cursor = close + 1;
    }
    String::from_utf8(out).expect("valid utf-8 preserved by byte-level blanking")
}

/// `(суффикс пути, подстрока строки)` — кириллица тут вне области фазы 9
/// (перевод текстов, уходящих модели): UI-сигналы, дефолтные UI-подписи или
/// принятое значение-синоним, а не проза для модели. Список сознательно
/// узкий и привязан к конкретным строкам — новая кириллица в этих файлах
/// потребует либо перевода, либо явного добавления сюда с причиной.
const ALLOWED: &[(&str, &str)] = &[
    // Стоп-слова русского запроса в проверке релевантности выдачи: это
    // данные фильтра, а не текст для модели — перевести их нельзя, они
    // и должны быть на языке запроса.
    ("src/agent/tools/web/search.rs", "\"или\" | \"как\" | \"что\""),
    // Голосовой ввод (запись/распознавание/выбор ASR-модели) — сигналы
    // `session.error()`/`refine_error`, отрисовываются в voice_fab-панели
    // (src/components/voice_fab/**), это отдельная UI-фаза i18n, не тексты
    // для модели.
    ("src/agent/audio.rs", "Настройки → Аудио модели"),
    ("src/agent/audio.rs", "Не удалось начать запись"),
    ("src/agent/audio.rs", "Не удалось остановить запись"),
    ("src/agent/audio.rs", "ASR-модель выгружена"),
    ("src/agent/audio.rs", "Не удалось распознать речь"),
    ("src/agent/audio.rs", "Ошибка распознавания"),
    ("src/agent/audio.rs", "Не выбрана ASR-модель"),
    ("src/agent/audio.rs", "Не указан путь к модели"),
    ("src/agent/audio.rs", "Не удалось загрузить модель"),
    ("src/agent/voice_refine.rs", "постобработка недоступна"),
    ("src/agent/voice_refine.rs", "занята генерацией"),
    ("src/agent/voice_refine.rs", "Постобработка недоступна"),
    // UI-лейблы вложений/аватара в чате (message_bubble, чипы) — не текст
    // для модели.
    ("src/agent/state.rs", "\"Изображение\""),
    ("src/agent/state.rs", "\"Видео\""),
    ("src/agent/state.rs", "\"Аудио\""),
    ("src/agent/state.rs", "\"Документ\""),
    ("src/agent/state.rs", "\"Файл\""),
    ("src/agent/state.rs", "\"Вы\""),
    ("src/agent/state.rs", "\"ВЫ\""),
    ("src/agent/state.rs", "\"Ассистент\""),
    // Дефолтный заголовок чата (сверяется при миграции старых файлов с
    // `registry::create_new`, UI-строка) и подпись «N вложений» в превью
    // списка чатов — обе не уходят модели.
    ("src/agent/storage.rs", "\"Новый чат\""),
    ("src/agent/storage.rs", "\"вложений\""),
    ("src/agent/storage.rs", "\"вложения\""),
    // Сигнал ошибки над полем «Отредактированный текст» — тот же UI-слой,
    // что и голосовой ввод выше.
    ("src/agent/streaming_asr.rs", "не удалось запустить drainer"),
    // Дефолтный заголовок служебной вкладки нодового редактора, которую
    // видит пользователь (см. `EditorWorkspace::ensure_agent_tab`) — не
    // текст для модели.
    ("src/agent/tools/pipelines.rs", "\"Агент\""),
    // Принятое значение-синоним true/false от модели — alias enum-значения,
    // а не проза для перевода (наравне с "true"/"1"/"yes" в том же match).
    ("src/agent/tools/system.rs", "\"да\""),
    // web_fetch — HTTP-хелпер kb-ingest пайплайна (src/kb/runner.rs):
    // ошибки уходят в UI-нотификацию (`app.notifications.error`), не
    // модели. `src/kb/**` — отдельная фаза i18n, не эта.
    ("src/agent/tools/web_fetch.rs", "HTTP-клиента"),
    ("src/agent/tools/web_fetch.rs", "сетевая ошибка"),
    ("src/agent/tools/web_fetch.rs", "чтения тела"),
    ("src/agent/tools/web_fetch.rs", "неподдерживаемый content-type"),
    // Notification-снекбар и `ctx.error` автокомпакта (UI прогресс/ошибка
    // сжатия контекста, `app.notifications.*`) — не текст для модели; сам
    // summary-запрос модели (`SUMMARY_SYSTEM_PROMPT`) уже на английском.
    ("src/syn_chat/compact.rs", "Модель не загружена"),
    ("src/syn_chat/compact.rs", "Сжатие контекста"),
    ("src/syn_chat/compact.rs", "Не удалось сжать контекст"),
    ("src/syn_chat/compact.rs", "Контекст сжат"),
];

#[test]
fn agent_and_syn_chat_texts_have_no_cyrillic() {
    let root = repo_root();

    let mut files = Vec::new();
    agent_rs_files(&root.join("src/agent"), &mut files);
    for rel in [
        "src/syn_chat/system_prompt.rs",
        "src/syn_chat/compact.rs",
        "src/syn_chat/channel_parser.rs",
        "src/syn_chat/tool_parser.rs",
    ] {
        files.push(root.join(rel));
    }
    files.sort();
    assert!(
        files.len() > 20,
        "найдено всего {} файлов — сборщик списка сломался",
        files.len()
    );

    let mut violations: Vec<String> = Vec::new();

    for f in &files {
        let raw = std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{f:?} не читается: {e}"));
        let prod_only = drop_test_module(&raw);
        let no_comments = strip_comments(prod_only);
        let cleaned = strip_diagnostic_calls(&no_comments);

        let rel = f.strip_prefix(&root).unwrap_or(f).display().to_string();
        for (lineno, line) in cleaned.lines().enumerate() {
            if !has_cyrillic(line) {
                continue;
            }
            let allowed = ALLOWED
                .iter()
                .any(|(path, needle)| rel.ends_with(path) && line.contains(needle));
            if !allowed {
                violations.push(format!("{rel}:{} — {}", lineno + 1, line.trim()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "кириллица вне тестов/комментариев/диагностики и вне ALLOWED — фаза \
         9 требует, чтобы тексты для модели были на английском:\n  {}",
        violations.join("\n  ")
    );
}

#[test]
fn default_system_prompt_is_english() {
    assert!(
        !has_cyrillic(DEFAULT_SYSTEM_PROMPT),
        "DEFAULT_SYSTEM_PROMPT содержит кириллицу"
    );
    // Сохранность контракта: правило действия и tool `web` остаются на месте.
    assert!(DEFAULT_SYSTEM_PROMPT.contains("ACTION RULE"));
    assert!(DEFAULT_SYSTEM_PROMPT.contains("`web`"));
}

#[test]
fn default_voice_refine_prompt_is_english() {
    assert!(
        !has_cyrillic(DEFAULT_VOICE_REFINE_PROMPT),
        "DEFAULT_VOICE_REFINE_PROMPT содержит кириллицу"
    );
}
