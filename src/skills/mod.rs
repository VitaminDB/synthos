//! Скилы (skills) — текстовые инструкции в Markdown, которые модель может
//! получить через tool `autoskill`. Каждый скил — отдельный `.md` файл в
//! `~/.config/synthos/skills/` с TOML-frontmatter (`name`, `description`) и
//! markdown-телом.
//!
//! Хранилище простое: один файл на скил, slug = id (имя файла без `.md`).
//! Это удобно редактировать вручную, версионировать через git и шарить
//! между установками. Подробное API IO — в [`storage`].

pub mod storage;

use std::collections::HashSet;
use std::path::PathBuf;

pub use storage::{
    SkillError, create, delete, load_all, load_one, save, skills_dir, update_meta,
};

/// Описание одного скила. `id` — slug, имя файла без `.md`. `content` —
/// markdown-тело без frontmatter'а.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub content: String,
}

impl Skill {
    /// Полный путь файла на диске (`<skills_dir>/<id>.md`).
    pub fn path(&self) -> PathBuf {
        skills_dir().join(format!("{}.md", self.id))
    }
}

/// Превращает произвольное имя в filesystem-safe slug. Транслит кириллицы,
/// прочее не-ASCII выкидывается. Пустой результат → `"skill"`. На коллизии
/// (slug уже занят в `taken`) добавляется `-2`, `-3` и т.д.
pub fn make_slug(name: &str, taken: &HashSet<String>) -> String {
    let translit = transliterate(name);
    let mut out = String::with_capacity(translit.len());
    let mut last_dash = false;
    for ch in translit.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("skill");
    }

    if !taken.contains(&out) {
        return out;
    }
    let base = out.clone();
    let mut n = 2usize;
    loop {
        let candidate = format!("{}-{}", base, n);
        if !taken.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Простейшая транслит-таблица RU→EN. Минимум — чтобы slug был читаемым,
/// без сторонних крейтов. Незнакомые символы (китайский, эмодзи) дропаются
/// в `make_slug` через ASCII-фильтр.
fn transliterate(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            'а' | 'А' => out.push('a'),
            'б' | 'Б' => out.push('b'),
            'в' | 'В' => out.push('v'),
            'г' | 'Г' => out.push('g'),
            'д' | 'Д' => out.push('d'),
            'е' | 'Е' | 'ё' | 'Ё' => out.push('e'),
            'ж' | 'Ж' => out.push_str("zh"),
            'з' | 'З' => out.push('z'),
            'и' | 'И' | 'й' | 'Й' => out.push('i'),
            'к' | 'К' => out.push('k'),
            'л' | 'Л' => out.push('l'),
            'м' | 'М' => out.push('m'),
            'н' | 'Н' => out.push('n'),
            'о' | 'О' => out.push('o'),
            'п' | 'П' => out.push('p'),
            'р' | 'Р' => out.push('r'),
            'с' | 'С' => out.push('s'),
            'т' | 'Т' => out.push('t'),
            'у' | 'У' => out.push('u'),
            'ф' | 'Ф' => out.push('f'),
            'х' | 'Х' => out.push('h'),
            'ц' | 'Ц' => out.push_str("ts"),
            'ч' | 'Ч' => out.push_str("ch"),
            'ш' | 'Ш' => out.push_str("sh"),
            'щ' | 'Щ' => out.push_str("sch"),
            'ъ' | 'Ъ' | 'ь' | 'Ь' => {}
            'ы' | 'Ы' => out.push('y'),
            'э' | 'Э' => out.push('e'),
            'ю' | 'Ю' => out.push_str("yu"),
            'я' | 'Я' => out.push_str("ya"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_basic() {
        let taken = HashSet::new();
        assert_eq!(make_slug("Hello World", &taken), "hello-world");
        assert_eq!(make_slug("UPPER case", &taken), "upper-case");
    }

    #[test]
    fn slug_translit() {
        let taken = HashSet::new();
        assert_eq!(make_slug("Приветствие", &taken), "privetstvie");
        assert_eq!(make_slug("Возврат средств", &taken), "vozvrat-sredstv");
    }

    #[test]
    fn slug_collisions() {
        let mut taken = HashSet::new();
        let a = make_slug("Test", &taken);
        assert_eq!(a, "test");
        taken.insert(a);
        let b = make_slug("Test", &taken);
        assert_eq!(b, "test-2");
        taken.insert(b);
        assert_eq!(make_slug("Test", &taken), "test-3");
    }

    #[test]
    fn slug_empty_fallbacks() {
        let taken = HashSet::new();
        assert_eq!(make_slug("", &taken), "skill");
        assert_eq!(make_slug("!!!", &taken), "skill");
        assert_eq!(make_slug("中文", &taken), "skill");
    }

    #[test]
    fn slug_strips_punctuation() {
        let taken = HashSet::new();
        assert_eq!(make_slug("file/path/name", &taken), "file-path-name");
        assert_eq!(make_slug("a___b---c", &taken), "a-b-c");
    }
}
