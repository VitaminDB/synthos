//! Карточка доски в буфере обмена.
//!
//! «Что копировать» выбирается при копировании (подменю «Копировать ▸»
//! карточки; Ctrl+C — карточка целиком), а «в каком виде» — местом
//! вставки: в буфер кладутся сразу несколько форматов, и каждый получатель
//! берёт свой. Терминал и редакторы — `text/plain`: текст карточки, где
//! вложения — абсолютные пути. Файловый менеджер и мессенджеры —
//! `text/uri-list`, но только у «Вложения как файлы»: иначе Telegram
//! вставил бы файлы вместо текста. Сами «Заметки» — карточку целиком:
//! последняя скопированная запоминается в процессе вместе со своим
//! текстом, и вставка на доску по совпадению текста берёт карточку со
//! всеми полями и вложениями; чужой текст становится новой карточкой.
//!
//! Вложения живут в бандле, поэтому пути ведут в папку выгрузки
//! [`media::export_card_files`]: исходные имена, жёсткие ссылки на
//! распакованный кэш, только чтение.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use syngui::prelude::*;
use syngui::widgets::input::document_editor::{BlockId, ClipboardKey};

use super::super::media;
use super::super::state::{LiveObject, NotesCtx};
use super::model::{item_id, DropSpot, KanbanCard, Repeat};
use super::{BoardEnv, KanbanHandle};

/// Что из карточки кладётся в буфер.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyKind {
    /// Заголовок, свойства, текст и список вложений путями.
    Full,
    TitleBody,
    /// Заголовок, текст и список вложений путями (без свойств).
    TitleBodyPaths,
    /// Заголовок и список всех вложений путями — и картинок из текста.
    TitlePaths,
    /// Заголовок как есть, без `###`.
    Title,
    Body,
    /// Пути вложений одной строкой, в кавычках для шелла.
    Paths,
    /// Вложения как файлы (`text/uri-list`) + пути текстом.
    Files,
}

impl CopyKind {
    /// В порядке пунктов меню.
    pub const ALL: [CopyKind; 8] = [
        CopyKind::Full,
        CopyKind::TitleBody,
        CopyKind::TitleBodyPaths,
        CopyKind::TitlePaths,
        CopyKind::Title,
        CopyKind::Body,
        CopyKind::Paths,
        CopyKind::Files,
    ];

    pub fn key(self) -> &'static str {
        match self {
            CopyKind::Full => "full",
            CopyKind::TitleBody => "title_body",
            CopyKind::TitleBodyPaths => "title_body_paths",
            CopyKind::TitlePaths => "title_paths",
            CopyKind::Title => "title",
            CopyKind::Body => "body",
            CopyKind::Paths => "paths",
            CopyKind::Files => "files",
        }
    }

    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }

    pub fn label(self) -> String {
        match self {
            CopyKind::Full => tr!("notes.kanban.copy.full"),
            CopyKind::TitleBody => tr!("notes.kanban.copy.title_body"),
            CopyKind::TitleBodyPaths => tr!("notes.kanban.copy.title_body_paths"),
            CopyKind::TitlePaths => tr!("notes.kanban.copy.title_paths"),
            CopyKind::Title => tr!("notes.kanban.copy.title"),
            CopyKind::Body => tr!("notes.kanban.copy.body"),
            CopyKind::Paths => tr!("notes.kanban.copy.paths"),
            CopyKind::Files => tr!("notes.kanban.copy.files"),
        }
    }

    /// Без вложений вид пуст либо совпадает с таким же без путей.
    pub fn needs_files(self) -> bool {
        matches!(self, CopyKind::TitleBodyPaths | CopyKind::TitlePaths | CopyKind::Paths | CopyKind::Files)
    }
}

/// Файлы вложений на диске: ссылка `asset:…` → путь.
pub type FilePaths = HashMap<String, PathBuf>;

/// Все вложения карточки по порядку — `(ссылка, имя)`: файлы со скрепкой,
/// затем картинки из содержимого (`asset:<sha>.<ext>` в markdown); без
/// повторов.
pub fn card_assets(card: &KanbanCard) -> Vec<(String, String)> {
    let mut out = file_entries(card);
    for url in body_assets(&card.md) {
        if !out.iter().any(|(u, _)| *u == url) {
            let name = url.strip_prefix("asset:").unwrap_or(&url).to_string();
            out.push((url, name));
        }
    }
    out
}

/// Ссылки `asset:<sha>.<ext>` в тексте, по порядку и без повторов.
pub fn body_assets(md: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = md;
    while let Some(i) = rest.find("asset:") {
        let tail = &rest[i..];
        let end = tail.find(|c: char| !(c.is_ascii_alphanumeric() || c == ':' || c == '.')).unwrap_or(tail.len());
        // Точка в конце предложения — не часть ссылки.
        let url = tail[..end].trim_end_matches('.');
        if media::parse_asset_url(url).is_some() && !out.iter().any(|u| u == url) {
            out.push(url.to_string());
        }
        rest = &tail[end..];
    }
    out
}

/// Текст с `asset:…` → абсолютными путями выгрузки.
pub fn with_paths(md: &str, files: &FilePaths) -> String {
    let mut out = md.to_string();
    for (url, path) in files {
        out = out.replace(url.as_str(), &path.display().to_string());
    }
    out
}

/// Пути вложений карточки (в порядке [`card_assets`]), что есть на диске.
pub fn card_paths(card: &KanbanCard, files: &FilePaths) -> Vec<PathBuf> {
    card_assets(card).into_iter().filter_map(|(url, _)| files.get(&url).cloned()).collect()
}

/// Путь для шелла: как есть, если экранировать нечего, иначе в одинарных
/// кавычках.
pub fn shell_quote(p: &Path) -> String {
    let s = p.display().to_string();
    let plain = !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || "/._-+,:@%=".contains(c));
    if plain { s } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

/// Текст карточки в виде `kind`; `column` — имя её колонки, `files` — пути
/// выгруженных вложений.
pub fn card_text(card: &KanbanCard, column: &str, kind: CopyKind, files: &FilePaths) -> String {
    let title = card.title.trim();
    let heading = if title.is_empty() { String::new() } else { format!("### {title}") };
    let body = with_paths(card.md.trim(), files);
    match kind {
        CopyKind::Title => title.to_string(),
        CopyKind::Body => body,
        CopyKind::TitleBody => join_blocks([heading, body]),
        CopyKind::Full => join_blocks([heading, props_line(card, column), body, attachments_list(&file_entries(card), files)]),
        CopyKind::TitleBodyPaths => join_blocks([heading, body, attachments_list(&file_entries(card), files)]),
        // Текста нет — картинки из него перечисляются вместе с файлами.
        CopyKind::TitlePaths => join_blocks([heading, attachments_list(&card_assets(card), files)]),
        CopyKind::Paths | CopyKind::Files => {
            card_paths(card, files).iter().map(|p| shell_quote(p)).collect::<Vec<_>>().join(" ")
        }
    }
}

/// Непустые куски через пустую строку.
fn join_blocks<const N: usize>(parts: [String; N]) -> String {
    parts.into_iter().filter(|p| !p.trim().is_empty()).collect::<Vec<_>>().join("\n\n")
}

/// «Колонка: … · Приоритет: … · Метки: … · Срок: …» — заполненные поля.
fn props_line(card: &KanbanCard, column: &str) -> String {
    let mut parts = Vec::new();
    if !column.trim().is_empty() {
        parts.push(format!("{}: {}", tr!("notes.kanban.copy.column"), column.trim()));
    }
    if let Some(p) = card.priority {
        parts.push(format!("{}: {}", tr!("notes.kanban.priority"), tr!(&p.i18n_key())));
    }
    if !card.tags.is_empty() {
        parts.push(format!("{}: {}", tr!("notes.kanban.tags"), card.tags.join(", ")));
    }
    if let Some(due) = &card.due {
        parts.push(format!("{}: {due}", tr!("notes.kanban.due")));
    }
    let span = card.span_text();
    if !span.is_empty() {
        parts.push(format!("{}: {span}", tr!("notes.kanban.planned")));
    }
    if card.repeat != Repeat::None {
        let repeat = syngui::i18n::tr(&format!("notes.calendar.repeat.{}", card.repeat.key()));
        parts.push(format!("{}: {repeat}", tr!("notes.kanban.repeat")));
    }
    if let Some(done) = &card.done {
        parts.push(format!("{}: {done}", tr!("notes.kanban.done_at")));
    }
    parts.join(" · ")
}

/// Вложения со скрепкой — `(ссылка, имя)`, без картинок из текста.
fn file_entries(card: &KanbanCard) -> Vec<(String, String)> {
    card.files.iter().map(|f| (f.url.clone(), f.label())).collect()
}

/// «Вложения:» и по пути на строку; файл, которого нет на диске, — именем.
fn attachments_list(entries: &[(String, String)], files: &FilePaths) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = format!("{}:", tr!("notes.kanban.copy.attachments"));
    for (url, name) in entries {
        out.push_str("\n- ");
        match files.get(url) {
            Some(p) => out.push_str(&p.display().to_string()),
            None => out.push_str(name),
        }
    }
    out
}

// ─── Своя карточка в буфере ───────────────────────────────────────────────

/// Последняя скопированная карточка, её текст в буфере и проект-источник.
struct Stash {
    text: String,
    card: KanbanCard,
    project: PathBuf,
}

static STASH: Mutex<Option<Stash>> = Mutex::new(None);

pub fn remember(text: &str, card: &KanbanCard, project: &Path) {
    *STASH.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Stash { text: text.to_string(), card: card.clone(), project: project.to_path_buf() });
}

/// Карточка последней копии, если буфер всё ещё держит её текст.
pub fn recall(clipboard: &str) -> Option<(KanbanCard, PathBuf)> {
    let guard = STASH.lock().unwrap_or_else(|e| e.into_inner());
    let s = guard.as_ref()?;
    (s.text.trim_end() == clipboard.trim_end()).then(|| (s.card.clone(), s.project.clone()))
}

/// Копия карточки для вставки: новый id, без штампов — их поставит
/// доводка доски по месту вставки.
pub fn fresh_copy(card: &KanbanCard) -> KanbanCard {
    let mut c = card.clone();
    c.id = item_id("k");
    c.created = None;
    c.done = None;
    c
}

/// Чужой текст → новая карточка: строка-заголовок (или единственная
/// простая строка) — заголовок, остальное — содержимое.
pub fn card_from_text(text: &str) -> Option<KanbanCard> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (title, body) = super::split_block(text);
    let mut c = KanbanCard::new(item_id("k"), String::new());
    c.title = title;
    c.md = body;
    Some(c)
}

// ─── Приложение: буфер, проект, доски на странице ─────────────────────────

/// Карточку — в буфер в виде `kind`. `false` — копировать нечего.
pub fn copy_card(project: &Path, card: &KanbanCard, column: &str, kind: CopyKind) -> bool {
    let files = media::export_card_files(project, card);
    let text = card_text(card, column, kind, &files);
    if text.trim().is_empty() {
        return false;
    }
    let mut formats: Vec<(&str, Vec<u8>)> = Vec::new();
    let paths = card_paths(card, &files);
    if kind == CopyKind::Files && !paths.is_empty() {
        let uris = syngui::clipboard::uri_list(&paths);
        formats.push(("x-special/gnome-copied-files", format!("copy\n{}", uris.replace("\r\n", "\n")).into_bytes()));
        formats.push(("text/uri-list", uris.into_bytes()));
    }
    let formats: Vec<(&str, &[u8])> = formats.iter().map(|(m, d)| (*m, d.as_slice())).collect();
    syngui::clipboard::copy_rich(&text, &formats);
    remember(&text, card, project);
    true
}

/// Буфер → карточка для вставки в проект `project`: своя копия целиком
/// (вложения из другого проекта переезжают), иначе новая из текста.
pub fn paste_card(project: &Path) -> Option<KanbanCard> {
    let text = syngui::clipboard::paste()?;
    if let Some((card, from)) = recall(&text) {
        if from != project {
            carry_assets(&from, project, &card);
        }
        return Some(fresh_copy(&card));
    }
    card_from_text(&text)
}

/// Вложения карточки из проекта `from` — в проект `to` (sha те же, ссылки
/// остаются верными).
fn carry_assets(from: &Path, to: &Path, card: &KanbanCard) {
    for (url, _) in card_assets(card) {
        let Some(name) = media::parse_asset_url(&url) else { continue };
        if media::assets_cache_dir(to).join(name).is_file() {
            continue;
        }
        let Some(bytes) = media::asset_file(from, &url).and_then(|p| std::fs::read(p).ok()) else { continue };
        let ext = name.rsplit('.').next().unwrap_or("bin");
        media::ingest_bytes(to, bytes, ext);
    }
}

/// Карточку доски — в буфер в виде `kind`.
pub fn copy_by_id(env: &BoardEnv, handle: &KanbanHandle, id: &str, kind: CopyKind) -> bool {
    let Some(card) = handle.card(id) else { return false };
    let column = handle.lock().column_name(&card.column);
    (env.copy_card)(&card, &column, kind);
    true
}

/// Карточку из буфера — в место `spot`, и выбрать её.
pub fn paste_at(env: &BoardEnv, handle: &KanbanHandle, spot: &DropSpot) -> bool {
    let Some(card) = (env.paste_card)() else { return false };
    let id = card.id.clone();
    handle.insert_card(card, spot);
    handle.select(Some(&id));
    true
}

/// Ctrl+C / Ctrl+V над выделенным блоком доски на странице: при выбранной
/// на ней карточке буфер работает с карточкой, а не с блоком
/// `![[kanban:…]]` целиком.
pub fn page_clipboard_key(ctx: NotesCtx, key: ClipboardKey, blocks: &[BlockId]) -> bool {
    let [block] = blocks else { return false };
    let Some(page) = ctx.active_page() else { return false };
    let Some(md) = page.handle.block_markdown(*block) else { return false };
    let Some(board) = embedded_board(&md) else { return false };
    let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &board) else { return false };
    let Some(selected) = handle.selected.get_untracked() else { return false };
    let env = super::env(ctx);
    match key {
        ClipboardKey::Copy => copy_by_id(&env, &handle, &selected, CopyKind::Full),
        ClipboardKey::Paste => {
            let spot = handle.lock().spot_after(&selected);
            if let Some(spot) = spot {
                paste_at(&env, &handle, &spot);
            }
            true
        }
        ClipboardKey::Cut => false,
    }
}

/// id доски во врезке `![[kanban:<id>]]{…}`.
fn embedded_board(md: &str) -> Option<String> {
    let rest = md.trim().strip_prefix("![[kanban:")?;
    let (target, _) = rest.split_once("]]")?;
    let id = target.split('|').next()?.trim();
    (!id.is_empty()).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::model::{CardFile, Priority};
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const SHB: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    fn ctx() {
        syngui::i18n::register_catalogs(&crate::i18n::CATALOGS);
        syngui::i18n::set_language(syngui::i18n::Lang::new("ru"));
    }

    fn sample() -> (KanbanCard, FilePaths) {
        let mut c = KanbanCard::new("k1".into(), "c1".into());
        c.title = "Импорт Excel".into();
        c.md = format!("- [ ] разобрать формат\n\n![скрин](asset:{SHB}.png)");
        c.priority = Some(Priority::High);
        c.tags = vec!["backend".into(), "excel".into()];
        c.due = Some("2026-09-15".into());
        c.files = vec![CardFile::new(format!("asset:{SHA}.pdf"), "спецификация v2.pdf")];
        let files: FilePaths = [
            (format!("asset:{SHA}.pdf"), PathBuf::from("/tmp/x/k1/спецификация v2.pdf")),
            (format!("asset:{SHB}.png"), PathBuf::from(format!("/tmp/x/k1/{SHB}.png"))),
        ]
        .into();
        (c, files)
    }

    #[test]
    fn body_assets_are_found_once_without_trailing_dot() {
        let md = format!("см. asset:{SHA}.png. и ![](asset:{SHA}.png) и asset:короткий.png и asset:{SHB}.pdf");
        assert_eq!(body_assets(&md), [format!("asset:{SHA}.png"), format!("asset:{SHB}.pdf")]);
        let (c, _) = sample();
        let names: Vec<String> = card_assets(&c).into_iter().map(|(_, n)| n).collect();
        assert_eq!(names, ["спецификация v2.pdf".to_string(), format!("{SHB}.png")]);
    }

    #[test]
    fn full_text_has_props_body_with_paths_and_attachments() {
        ctx();
        let (c, files) = sample();
        let text = card_text(&c, "В работе", CopyKind::Full, &files);
        assert_eq!(
            text,
            format!(
                "### Импорт Excel\n\n\
                 Колонка: В работе · Приоритет: Высокий · Метки: backend, excel · Срок: 2026-09-15\n\n\
                 - [ ] разобрать формат\n\n![скрин](/tmp/x/k1/{SHB}.png)\n\n\
                 Вложения:\n- /tmp/x/k1/спецификация v2.pdf"
            )
        );
    }

    #[test]
    fn other_kinds() {
        ctx();
        let (c, files) = sample();
        assert_eq!(
            card_text(&c, "В работе", CopyKind::TitleBody, &files),
            format!("### Импорт Excel\n\n- [ ] разобрать формат\n\n![скрин](/tmp/x/k1/{SHB}.png)")
        );
        assert_eq!(
            card_text(&c, "В работе", CopyKind::TitleBodyPaths, &files),
            format!(
                "### Импорт Excel\n\n- [ ] разобрать формат\n\n![скрин](/tmp/x/k1/{SHB}.png)\n\n\
                 Вложения:\n- /tmp/x/k1/спецификация v2.pdf"
            )
        );
        // Без текста картинка из него — в общем списке вложений.
        assert_eq!(
            card_text(&c, "", CopyKind::TitlePaths, &files),
            format!("### Импорт Excel\n\nВложения:\n- /tmp/x/k1/спецификация v2.pdf\n- /tmp/x/k1/{SHB}.png")
        );
        for k in CopyKind::ALL {
            assert_eq!(CopyKind::parse(k.key()), Some(k));
        }
        assert_eq!(card_text(&c, "В работе", CopyKind::Title, &files), "Импорт Excel");
        assert!(card_text(&c, "", CopyKind::Body, &files).starts_with("- [ ] разобрать"));
        assert_eq!(
            card_text(&c, "", CopyKind::Paths, &files),
            format!("'/tmp/x/k1/спецификация v2.pdf' /tmp/x/k1/{SHB}.png")
        );
        // Пустая карточка — копировать нечего; без заголовка — без «###».
        let empty = KanbanCard::new("k2".into(), "c1".into());
        assert_eq!(card_text(&empty, "", CopyKind::Full, &FilePaths::new()), "");
        let mut no_title = empty.clone();
        no_title.md = "текст".into();
        assert_eq!(card_text(&no_title, "", CopyKind::TitleBody, &FilePaths::new()), "текст");
        // Вложение, не выгруженное на диск, — именем.
        let (c, _) = sample();
        assert!(card_text(&c, "", CopyKind::Full, &FilePaths::new()).ends_with("- спецификация v2.pdf"));
    }

    #[test]
    fn shell_quoting() {
        assert_eq!(shell_quote(Path::new("/a/файл-1.txt")), "/a/файл-1.txt");
        assert_eq!(shell_quote(Path::new("/a/b c.txt")), "'/a/b c.txt'");
        assert_eq!(shell_quote(Path::new("/a/it's")), r"'/a/it'\''s'");
    }

    #[test]
    fn own_copy_is_recalled_by_text_and_pasted_fresh() {
        let (c, _) = sample();
        remember("текст карточки\n", &c, Path::new("/p.syn"));
        assert!(recall("чужой текст").is_none());
        let (card, project) = recall("текст карточки").expect("своя копия");
        assert_eq!((card.id.as_str(), project.as_path()), ("k1", Path::new("/p.syn")));
        let mut done = card.clone();
        done.created = Some("2026-09-01".into());
        done.done = Some("2026-09-02".into());
        let fresh = fresh_copy(&done);
        assert_ne!(fresh.id, "k1");
        assert_eq!((fresh.created.as_deref(), fresh.done.as_deref()), (None, None));
        assert_eq!((fresh.title.as_str(), fresh.files.len()), ("Импорт Excel", 1));
    }

    #[test]
    fn foreign_text_becomes_a_card() {
        let c = card_from_text("## Задача\n\nописание").unwrap();
        assert_eq!((c.title.as_str(), c.md.as_str()), ("Задача", "описание"));
        assert_eq!(card_from_text("простая строка").unwrap().title, "простая строка");
        assert!(card_from_text("  \n").is_none());
    }

    #[test]
    fn board_id_from_embed() {
        assert_eq!(embedded_board("![[kanban:b1]]{h=340}\n").as_deref(), Some("b1"));
        assert_eq!(embedded_board("![[gantt:g1]]"), None);
        assert_eq!(embedded_board("# Заголовок"), None);
    }
}
