//! Каждый ключ `tr!`/`trn!` в исходниках обязан быть в `i18n/en.lang` и
//! `i18n/ru.lang`, а `ru` — совпадать с `en` по набору ключей и плейсхолдерам.
//!
//! Остальные каталоги проверяются мягче: неизвестный ключ — ошибка (опечатка),
//! отсутствующий — предупреждение со счётчиком. Без этого теста пропущенный
//! перевод виден только глазами: `tr!` молча возвращает сам ключ.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use syngui::i18n::format::placeholders;
use syngui::i18n::Catalog;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

const TR_CALLS: &[&str] = &["tr!(\"", "tr(\"", "tr_args(\"", "try_tr(\""];
const TRN_CALLS: &[&str] = &["trn!(\"", "trn(\"", "trn_args(\""];

fn collect(src: &str, markers: &[&str], into: &mut BTreeMap<String, Vec<String>>, file: &str) {
    for marker in markers {
        let mut rest = src;
        while let Some(pos) = rest.find(marker) {
            let after = &rest[pos + marker.len()..];
            let boundary_ok = pos == 0 || !rest.as_bytes()[pos - 1].is_ascii_alphanumeric();
            if let Some(end) = after.find('"') {
                if boundary_ok {
                    into.entry(after[..end].to_string()).or_default().push(file.to_string());
                }
                rest = &after[end..];
            } else {
                break;
            }
        }
    }
}

fn source_keys() -> (BTreeMap<String, Vec<String>>, BTreeMap<String, Vec<String>>) {
    let mut files = Vec::new();
    rust_files(&repo_root().join("src"), &mut files);
    let mut tr = BTreeMap::new();
    let mut trn = BTreeMap::new();
    for path in files {
        let src = std::fs::read_to_string(&path).unwrap();
        let name = path.strip_prefix(repo_root()).unwrap().display().to_string();
        collect(&src, TR_CALLS, &mut tr, &name);
        collect(&src, TRN_CALLS, &mut trn, &name);
    }
    (tr, trn)
}

fn catalogs() -> Vec<(String, Catalog)> {
    let dir = repo_root().join("i18n");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "lang") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path).unwrap();
        let catalog = Catalog::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        out.push((stem, catalog));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn catalog(tag: &str) -> Catalog {
    catalogs().into_iter().find(|(stem, _)| stem == tag).map(|(_, c)| c).unwrap()
}

fn has_plural(cat: &Catalog, key: &str) -> bool {
    cat.get(key).is_some() || cat.get(&format!("{key}.other")).is_some() || cat.get(&format!("{key}.one")).is_some()
}

fn plural_base(key: &str) -> &str {
    for suffix in [".zero", ".one", ".two", ".few", ".many", ".other"] {
        if let Some(base) = key.strip_suffix(suffix) {
            return base;
        }
    }
    key
}

#[test]
fn catalogs_parse_and_match_embedded_list() {
    let all = catalogs();
    assert_eq!(all.len(), synthos::i18n::CATALOGS.len(), "i18n/*.lang и CATALOGS разошлись");
    for (stem, cat) in &all {
        assert_eq!(cat.tag.tag(), stem, "@tag должен совпадать с именем файла");
        assert!(!cat.name.is_empty(), "{stem}: пустой @name");
    }
}

#[test]
fn source_keys_exist_in_en_and_ru() {
    let (tr, trn) = source_keys();
    assert!(!tr.is_empty(), "в исходниках не найдено ни одного tr!");
    for tag in ["en", "ru"] {
        let cat = catalog(tag);
        let mut missing: Vec<String> = Vec::new();
        for (key, files) in &tr {
            if cat.get(key).is_none() {
                missing.push(format!("{key}  ({})", files[0]));
            }
        }
        for (key, files) in &trn {
            if !has_plural(&cat, key) {
                missing.push(format!("{key} (plural)  ({})", files[0]));
            }
        }
        assert!(missing.is_empty(), "{tag}.lang: нет ключей:\n  {}", missing.join("\n  "));
    }
}

#[test]
fn ru_matches_en_keys_and_placeholders() {
    let en = catalog("en");
    let ru = catalog("ru");
    let en_keys: BTreeSet<&str> = en.keys().collect();
    let ru_keys: BTreeSet<&str> = ru.keys().collect();
    let only_en: Vec<&&str> = en_keys.difference(&ru_keys).collect();
    let only_ru: Vec<&&str> = ru_keys.difference(&en_keys).collect();
    assert!(only_en.is_empty() && only_ru.is_empty(), "ru vs en: только в en {only_en:?}, только в ru {only_ru:?}");
    for key in &en_keys {
        let a: BTreeSet<&str> = placeholders(en.get(key).unwrap()).into_iter().collect();
        let b: BTreeSet<&str> = placeholders(ru.get(key).unwrap()).into_iter().collect();
        assert_eq!(a, b, "плейсхолдеры ключа {key} различаются");
    }
}

#[test]
fn other_catalogs_have_no_unknown_keys() {
    let en = catalog("en");
    let en_keys: BTreeSet<String> = en.keys().map(|k| plural_base(k).to_string()).collect();
    for (stem, cat) in catalogs() {
        if stem == "en" || stem == "ru" {
            continue;
        }
        let unknown: Vec<&str> = cat.keys().filter(|k| !en_keys.contains(plural_base(k))).collect();
        assert!(unknown.is_empty(), "{stem}.lang: ключей нет в en: {unknown:?}");
        let missing = en_keys.iter().filter(|k| !has_plural(&cat, k)).count();
        if missing > 0 {
            eprintln!("warning: {stem}.lang не хватает {missing} из {} ключей", en_keys.len());
        }
        for key in cat.keys() {
            if let Some(reference) = en.get(key) {
                let a: BTreeSet<&str> = placeholders(reference).into_iter().collect();
                let b: BTreeSet<&str> = placeholders(cat.get(key).unwrap()).into_iter().collect();
                assert_eq!(a, b, "{stem}.lang: плейсхолдеры ключа {key} различаются");
            }
        }
    }
}
