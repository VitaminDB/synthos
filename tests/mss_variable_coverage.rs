//! Каждая `var(--name)` в `styles/**.mss` обязана быть где-то объявлена.
//!
//! Зачем тест: парсер MSS не ругается на неизвестную переменную — свойство
//! просто не применяется. Для `background-color` это значит прозрачный фон,
//! и обнаружить такое можно только глазами.
//!
//! Так в приложении молча не работали: кнопка загрузки модели
//! (`var(--error-container)` — переменной не существует, кнопка выглядела
//! полностью плоской), hover на вкладках терминала и нодного редактора,
//! фон активной вкладки, hover в дереве файлов, подложка «Размышлений» в
//! ленте чата и карточка sampling'а — пятнадцать мест на пять переменных.
//!
//! Источники объявлений три: сами `.mss`, `SynthosTheme::to_mss` +
//! `accent_override_mss` в `theme_data.rs` и блок шрифтов в `lib.rs`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Заменяет `/* ... */` пробелами, сохраняя переводы строк: так номера
/// строк в отчёте остаются настоящими, а закомментированные примеры вида
/// `var(--x, default)` не попадают в выборку.
fn blank_comments(src: &str) -> String {
    let bytes: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut in_comment = false;
    while i < bytes.len() {
        if !in_comment && bytes[i] == '/' && bytes.get(i + 1) == Some(&'*') {
            in_comment = true;
            out.push_str("  ");
            i += 2;
            continue;
        }
        if in_comment && bytes[i] == '*' && bytes.get(i + 1) == Some(&'/') {
            in_comment = false;
            out.push_str("  ");
            i += 2;
            continue;
        }
        if in_comment {
            out.push(if bytes[i] == '\n' { '\n' } else { ' ' });
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    out
}

/// Имена вида `--foo-bar`, за которыми следует `:` — то есть объявления.
fn declarations(src: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == '-' && chars[i + 1] == '-' {
            let start = i;
            let mut j = i + 2;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '-') {
                j += 1;
            }
            // Между именем и `:` допускаем пробелы.
            let mut k = j;
            while k < chars.len() && chars[k] == ' ' {
                k += 1;
            }
            if k < chars.len() && chars[k] == ':' && j > start + 2 {
                out.insert(chars[start..j].iter().collect::<String>());
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

/// `(имя, файл, строка)` для каждого `var(--name)`.
fn usages(src: &str, file: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (lineno, line) in src.lines().enumerate() {
        let mut rest = line;
        while let Some(pos) = rest.find("var(") {
            let tail = &rest[pos + 4..];
            let name: String = tail
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            // Только полная форма `var(--name)`: `var(--name, fallback)`
            // парсер тоже не поддерживает, но это отдельный разговор.
            if name.starts_with("--") {
                let after = tail.trim_start()[name.len()..].trim_start();
                if after.starts_with(')') {
                    out.push((name, format!("{}:{}", file.display(), lineno + 1)));
                }
            }
            rest = &rest[pos + 4..];
        }
    }
    out
}

fn mss_files(dir: &Path, acc: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            mss_files(&p, acc);
        } else if p.extension().is_some_and(|x| x == "mss") {
            acc.push(p);
        }
    }
}

#[test]
fn every_mss_variable_is_declared() {
    let root = repo_root();

    let mut files = Vec::new();
    mss_files(&root.join("styles"), &mut files);
    files.sort();
    assert!(
        files.len() > 20,
        "найдено всего {} .mss файлов — сборщик списка сломался",
        files.len()
    );

    let mut declared = HashSet::new();
    let mut all_usages = Vec::new();
    for f in &files {
        let src = blank_comments(&std::fs::read_to_string(f).expect("mss не читается"));
        declared.extend(declarations(&src));
        let rel = f.strip_prefix(&root).unwrap_or(f);
        all_usages.extend(usages(&src, rel));
    }

    // Переменные, которые генерируются из Rust, а не лежат в .mss.
    for rs in ["src/pages/settings/theme_data.rs", "src/lib.rs"] {
        let src = std::fs::read_to_string(root.join(rs)).expect("rs не читается");
        declared.extend(declarations(&src));
    }

    assert!(
        declared.len() > 100,
        "разобрано всего {} объявлений — парсер сломался",
        declared.len()
    );

    let missing: Vec<String> = all_usages
        .iter()
        .filter(|(name, _)| !declared.contains(name))
        .map(|(name, loc)| format!("{name} — {loc}"))
        .collect();

    assert!(
        missing.is_empty(),
        "{} использований необъявленных MSS-переменных (свойство молча не \
         применится):\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}
