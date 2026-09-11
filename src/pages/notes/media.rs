//! Медиа-адаптер заметок: вложения внутри бандла ↔ DocumentEditor.
//!
//! Вложения лежат в проекте File-чанками `notes/assets/<sha256>.<ext>`; в
//! markdown — ссылки `asset:<sha256>.<ext>`. Плеерам и картинкам нужен
//! файл на диске, поэтому при первом обращении чанк распаковывается в
//! кэш `~/.cache/synthos/notes-assets/<проект>/`. Ingest при дропе — sha256
//! + запись в бандл через автосейв (и сразу в кэш), затем
//! `handle.patch_media` на main. Старые `blob:`-ссылки (первая волна)
//! резолвятся в общий CAS чатов.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use sha2::{Digest, Sha256};
use syngui::prelude::*;
use syngui::widgets::input::document_editor::{DocMediaResolver, DocOp, MediaKind, ResolvedMedia};

use crate::syn_chat::attach::blobs;

use super::autosave;
use super::project;
use super::state::NotesCtx;

/// Потолок размера аудио-файла для синхронного декода волны.
const MAX_WAVEFORM_BYTES: u64 = 12 * 1024 * 1024;

pub struct NotesMediaResolver {
    pub project_path: PathBuf,
}

pub fn resolver(ctx: NotesCtx) -> Arc<NotesMediaResolver> {
    Arc::new(NotesMediaResolver { project_path: ctx.project_path.get_untracked() })
}

/// Кэш synthos: `$XDG_CACHE_HOME/synthos`, иначе `~/.cache/synthos`.
fn cache_root() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            home.join(".cache")
        });
    base.join("synthos")
}

/// Папка проекта в кэше: имя файла + хэш пути.
fn project_key(project_path: &Path) -> String {
    let mut h = DefaultHasher::new();
    project_path.hash(&mut h);
    let stem = project::project_title(project_path)
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>();
    format!("{stem}-{:08x}", h.finish() as u32)
}

/// Папка кэша распакованных вложений проекта.
pub fn assets_cache_dir(project_path: &Path) -> PathBuf {
    cache_root().join("notes-assets").join(project_key(project_path))
}

/// Папка выгрузок вложений карточек проекта (пути в буфере обмена).
pub fn export_dir(project_path: &Path) -> PathBuf {
    cache_root().join("notes-export").join(project_key(project_path))
}

/// Сколько живёт выгрузка карточки, которую больше не копировали.
const EXPORT_TTL: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);
/// Метка последней выгрузки в папке карточки: по её времени чистятся старые.
const EXPORT_STAMP: &str = ".copied";

/// Вложения карточки на диске под исходными именами — для путей в буфере
/// обмена: `~/.cache/synthos/notes-export/<проект>/<карточка>/<имя>`.
/// Файл — жёсткая ссылка на распакованный кэш (копия, если ссылка не
/// вышла) и только для чтения: правка «на месте» не испортит кэш. Папка
/// повторяет вложения ровно (лишнее от прошлых выгрузок убирается),
/// выгрузки старше недели удаляются. Картинки из содержимого — под
/// именем `<sha>.<ext>`. Ключ результата — ссылка `asset:…`.
pub fn export_card_files(project_path: &Path, card: &super::kanban::model::KanbanCard) -> HashMap<String, PathBuf> {
    let assets = super::kanban::clip::card_assets(card);
    let mut out = HashMap::new();
    if assets.is_empty() {
        return out;
    }
    let root = export_dir(project_path);
    prune_exports(&root);
    let dir = root.join(safe_file_name(&card.id));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("notes: папка выгрузки {} не создана: {e}", dir.display());
        return out;
    }
    let mut used = HashSet::new();
    for (url, name) in assets {
        let Some(src) = asset_file(project_path, &url) else { continue };
        let name = unique_file_name(&safe_file_name(&name), &mut used);
        let dst = dir.join(&name);
        if link_readonly(&src, &dst) {
            out.insert(url, dst);
        }
    }
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name != EXPORT_STAMP && !used.contains(&name) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let _ = std::fs::write(dir.join(EXPORT_STAMP), b"");
    out
}

/// Удалить выгрузки карточек, которые не копировали дольше [`EXPORT_TTL`].
fn prune_exports(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let now = std::time::SystemTime::now();
    for e in entries.flatten() {
        let path = e.path();
        let stamp = std::fs::metadata(path.join(EXPORT_STAMP)).or_else(|_| e.metadata()).and_then(|m| m.modified());
        let stale = stamp.ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > EXPORT_TTL);
        if stale && path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Имя файла без разделителей пути и ведущих точек (не скрытый, не `..`).
fn safe_file_name(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let s = s.trim_start_matches('.');
    if s.is_empty() { "file".to_string() } else { s.to_string() }
}

/// Имя, ещё не занятое в папке: `имя (2).ext`, `имя (3).ext`…
fn unique_file_name(name: &str, used: &mut HashSet<String>) -> String {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    let mut candidate = name.to_string();
    let mut n = 2;
    while used.contains(&candidate) {
        candidate = format!("{stem} ({n}){ext}");
        n += 1;
    }
    used.insert(candidate.clone());
    candidate
}

/// `dst` — тот же файл, что `src`: жёсткая ссылка либо копия, только чтение.
fn link_readonly(src: &Path, dst: &Path) -> bool {
    if same_file(src, dst) {
        return true;
    }
    let _ = std::fs::remove_file(dst);
    if std::fs::hard_link(src, dst).is_err() {
        if let Err(e) = std::fs::copy(src, dst) {
            log::warn!("notes: вложение не выгружено в {}: {e}", dst.display());
            return false;
        }
    }
    if let Ok(meta) = std::fs::metadata(dst) {
        let mut perm = meta.permissions();
        perm.set_readonly(true);
        let _ = std::fs::set_permissions(dst, perm);
    }
    true
}

#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(_a: &Path, _b: &Path) -> bool {
    false
}

/// `asset:<sha>.<ext>` → имя файла вложения.
pub fn parse_asset_url(url: &str) -> Option<&str> {
    let name = url.strip_prefix("asset:")?;
    let (sha, ext) = name.split_once('.')?;
    (sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()) && !ext.is_empty() && !name.contains('/'))
        .then_some(name)
}

/// `blob:<sha>.<ext>` → (sha, ext) — наследие первой волны.
fn parse_blob_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("blob:")?;
    let (sha, ext) = rest.split_once('.')?;
    (sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit())).then_some((sha, ext))
}

impl NotesMediaResolver {
    /// Файл вложения в кэше; распаковывается из бандла при первом обращении.
    fn extracted(&self, name: &str) -> Option<PathBuf> {
        let dir = assets_cache_dir(&self.project_path);
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
        let bytes = project::read_bytes(&self.project_path, &project::asset_path(name))?;
        std::fs::create_dir_all(&dir).ok()?;
        let tmp = dir.join(format!("{name}.tmp~"));
        std::fs::write(&tmp, bytes).ok()?;
        std::fs::rename(&tmp, &path).ok()?;
        Some(path)
    }
}

/// Файл вложения `asset:<sha>.<ext>` на диске (распаковка в кэш при первом
/// обращении) — для миниатюр и открытия вложений карточек.
pub fn asset_file(project_path: &Path, url: &str) -> Option<PathBuf> {
    let name = parse_asset_url(url)?;
    NotesMediaResolver { project_path: project_path.to_path_buf() }.extracted(name)
}

/// Файл с диска → вложение бандла → [`CardFile`] с исходным именем.
pub fn ingest_card_file(ctx: NotesCtx, file: &Path) -> Option<super::kanban::model::CardFile> {
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("notes: не прочитан {}: {e}", file.display());
            return None;
        }
    };
    let ext = file.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let url = ingest_bytes(&ctx.project_path.get_untracked(), bytes, &ext);
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Some(super::kanban::model::CardFile::new(url, name))
}

/// Диалог выбора файла для вложения карточки.
pub fn pick_card_file(ctx: NotesCtx) -> Option<super::kanban::model::CardFile> {
    let path = rfd::FileDialog::new().set_title(&tr!("notes.kanban.attach_file")).pick_file()?;
    ingest_card_file(ctx, &path)
}

/// Открыть вложение карточки: картинку — в окне просмотра, остальное —
/// системным приложением.
pub fn open_card_file(ctx: NotesCtx, file: &super::kanban::model::CardFile) {
    let Some(path) = asset_file(&ctx.project_path.get_untracked(), &file.url) else { return };
    if file.is_image() {
        ctx.viewer_file.set(Some((path, file.label())));
        ctx.viewer_open.set(true);
    } else {
        open_external(&path);
    }
}

/// Открыть файл системным приложением (`xdg-open`, иначе как url).
pub fn open_external(path: &Path) {
    #[cfg(target_os = "linux")]
    {
        if std::process::Command::new("xdg-open").arg(path).spawn().is_ok() {
            return;
        }
    }
    if let Err(e) = syngui::open_url(&format!("file://{}", path.display())) {
        log::warn!("notes: не удалось открыть {}: {e}", path.display());
    }
}

/// Окно просмотра картинки (вложение карточки): модальное, по центру,
/// картинка вписывается целиком.
pub fn image_viewer(ctx: NotesCtx) -> impl Widget {
    use syngui::widgets::overlay::FloatingWindow;
    use syngui::widgets::{Image, ImageFit};
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let open = ctx.viewer_open.get();
        let Some((path, name)) = ctx.viewer_file.get() else {
            return vec![Box::new(DecoratedBox::new())];
        };
        if !open {
            return vec![Box::new(DecoratedBox::new())];
        }
        let window = FloatingWindow::new(name)
            .icon(crate::icons::MI_IMAGE_ICON)
            .is_open(ctx.viewer_open)
            .size(syngui::core::Size::new(840.0, 640.0))
            .centered()
            .modal(true)
            .with_resizable(true)
            .closable(true)
            .child(
                DecoratedBox::new()
                    .class("notes-image-viewer")
                    .child(Image::new(path.display().to_string()).fit(ImageFit::Contain).class("notes-image-viewer-img")),
            )
            .class("notes-image-viewer-window");
        vec![Box::new(window)]
    })
}

impl DocMediaResolver for NotesMediaResolver {
    fn resolve(&self, url: &str) -> Option<ResolvedMedia> {
        let path = if let Some(name) = parse_asset_url(url) {
            self.extracted(name)?
        } else if let Some((sha, ext)) = parse_blob_url(url) {
            blobs::blob_path(sha, ext)
        } else if url.contains("://") || url.starts_with("pending:") {
            return None;
        } else {
            let p = Path::new(url);
            if !p.is_absolute() {
                return None;
            }
            p.to_path_buf()
        };
        path.is_file().then(|| ResolvedMedia {
            kind: MediaKind::detect(url, &Default::default()),
            path,
        })
    }

    fn pcm_bins(&self, url: &str, bins: usize) -> Option<Vec<f32>> {
        let resolved = self.resolve(url)?;
        static CACHE: OnceLock<Mutex<HashMap<String, Option<Vec<f32>>>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(url) {
            return hit.clone();
        }
        let computed = (|| {
            let size = std::fs::metadata(&resolved.path).ok()?.len();
            if size > MAX_WAVEFORM_BYTES {
                return None;
            }
            let buf = crate::pages::node_editor::nodes::decode::decode_file(&resolved.path).ok()?;
            Some(syngui::audio::compute_rms_bins(&buf.pcm, buf.channels, bins))
        })();
        cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(url.to_string(), computed.clone());
        computed
    }
}

/// Дроп файла в редактор: фоновый ingest в бандл + patch pending-блока.
pub fn ingest_dropped_file(ctx: NotesCtx, page_id: String, file: PathBuf, token: String) {
    let Some(page) = ctx.page(&page_id) else { return };
    let handle = page.handle;
    let project_path = ctx.project_path.get_untracked();
    std::thread::spawn(move || {
        let bytes = match std::fs::read(&file) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("notes: ingest {} не удался: {e}", file.display());
                return;
            }
        };
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let sha = format!("{:x}", hasher.finalize());
        let ext = file
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| "bin".to_string());
        let name = format!("{sha}.{ext}");
        // Сразу в кэш — плеер увидит файл до commit'а бандла.
        let dir = assets_cache_dir(&project_path);
        if std::fs::create_dir_all(&dir).is_ok() {
            let _ = std::fs::write(dir.join(&name), &bytes);
        }
        autosave::queue_bytes(&project::asset_path(&name), bytes);
        let url = format!("asset:{name}");
        syngui::async_runtime::run_on_main_thread(move || {
            if handle.patch_media(&token, &url) {
                ctx.bump_doc_epoch();
            }
        });
    });
}

/// Что предлагает выбрать диалог вставки медиа.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickKind {
    Image,
    Svg,
    File,
}

/// Байты → вложение проекта: sha256, кэш распакованных, очередь автосейва.
/// Возвращает ссылку `asset:<sha>.<ext>` для markdown.
pub fn ingest_bytes(project_path: &Path, bytes: Vec<u8>, ext: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha = format!("{:x}", hasher.finalize());
    let ext = ext.trim_start_matches('.').to_lowercase();
    let ext = if ext.is_empty() { "bin".to_string() } else { ext };
    let name = format!("{sha}.{ext}");
    // Сразу в кэш — картинка отрисуется до commit'а бандла.
    let dir = assets_cache_dir(project_path);
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join(&name), &bytes);
    }
    autosave::queue_bytes(&project::asset_path(&name), bytes);
    format!("asset:{name}")
}

/// Диалог выбора файла и вставка медиа-блока в место каретки (в свободной
/// раскладке — в точку правого клика).
pub fn pick_and_insert(ctx: NotesCtx, kind: PickKind) {
    let title = match kind {
        PickKind::Image => tr!("notes.menu.image"),
        PickKind::Svg => tr!("notes.menu.svg"),
        PickKind::File => tr!("notes.menu.file"),
    };
    let mut dlg = rfd::FileDialog::new().set_title(&title);
    dlg = match kind {
        PickKind::Image => dlg.add_filter(
            &tr!("notes.media.filter.image"),
            &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg"],
        ),
        PickKind::Svg => dlg.add_filter("SVG", &["svg"]),
        PickKind::File => dlg,
    };
    let Some(path) = dlg.pick_file() else { return };
    insert_file(ctx, &path);
}

/// Файл с диска → вложение бандла → медиа-блок документа.
pub fn insert_file(ctx: NotesCtx, file: &Path) {
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("notes: не прочитан {}: {e}", file.display());
            return;
        }
    };
    let ext = file.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let url = ingest_bytes(&ctx.project_path.get_untracked(), bytes, &ext);
    // Скобки в подписи разъехались бы с синтаксисом `![alt](url)`.
    let alt = file
        .file_stem()
        .map(|s| s.to_string_lossy().replace(['[', ']', '\n'], " "))
        .unwrap_or_default();
    ctx.doc_op(DocOp::InsertMarkdown(format!("![{alt}]({url})")));
}

/// SVG-разметка из буфера обмена → вложение проекта → картинка в документе.
/// `false` — в буфере не SVG (хост показывает подсказку).
pub fn insert_svg_from_clipboard(ctx: NotesCtx) -> bool {
    let Some(text) = syngui::clipboard::paste() else { return false };
    let trimmed = text.trim();
    if !looks_like_svg(trimmed) {
        return false;
    }
    let url = ingest_bytes(
        &ctx.project_path.get_untracked(),
        trimmed.as_bytes().to_vec(),
        "svg",
    );
    ctx.doc_op(DocOp::InsertMarkdown(format!("![svg]({url})")));
    true
}

/// Похож ли текст на SVG-разметку: корневой тег либо пролог/DOCTYPE перед
/// ним. Проверка нужна до записи вложения — иначе в проект попадал бы любой
/// скопированный текст.
pub fn looks_like_svg(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<svg")
        || ((t.starts_with("<?xml") || t.starts_with("<!DOCTYPE") || t.starts_with("<!--"))
            && t.contains("<svg"))
}

/// Все sha256 общего CAS, на которые ещё ссылаются страницы проекта
/// (`blob:` первой волны) — чтобы GC блобов чатов их не выбросил.
pub fn collect_blob_refs(referenced: &mut HashSet<String>) {
    let cfg = crate::config::AppConfig::load();
    let path = project::resolve_project_path(&cfg.notes_project_path);
    if !path.is_file() {
        return;
    }
    let tree = project::read_tree(&path);
    for node in tree.all() {
        if let Some(content) = project::read_text(&path, &project::page_path(&node.id)) {
            collect_blob_refs_in(&content, referenced);
        }
    }
}

fn collect_blob_refs_in(content: &str, referenced: &mut HashSet<String>) {
    let mut rest = content;
    while let Some(pos) = rest.find("blob:") {
        let tail = &rest[pos + 5..];
        let sha: String = tail.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        let sha_len = sha.len();
        if sha_len == 64 {
            referenced.insert(sha);
        }
        rest = &tail[sha_len.min(tail.len())..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_url_parsing() {
        let sha = "a".repeat(64);
        assert_eq!(parse_asset_url(&format!("asset:{sha}.mp4")), Some(format!("{sha}.mp4")).as_deref());
        assert!(parse_asset_url("asset:short.mp4").is_none());
        assert!(parse_asset_url(&format!("asset:{sha}")).is_none());
        assert!(parse_asset_url("https://x/y.mp4").is_none());
    }

    #[test]
    fn svg_sniffing() {
        assert!(looks_like_svg("<svg viewBox=\"0 0 10 10\"></svg>"));
        assert!(looks_like_svg("  \n<svg/>"));
        assert!(looks_like_svg("<?xml version=\"1.0\"?>\n<svg></svg>"));
        assert!(looks_like_svg("<!-- иконка -->\n<svg></svg>"));
        assert!(!looks_like_svg("просто текст"));
        assert!(!looks_like_svg("<html><body>svg</body></html>"));
        assert!(!looks_like_svg("<?xml version=\"1.0\"?><rss/>"));
    }

    #[test]
    fn blob_refs_collector() {
        let sha = "b".repeat(64);
        let md = format!("![v](blob:{sha}.mp4){{loop}} и ещё blob:{}", "c".repeat(64));
        let mut set = HashSet::new();
        collect_blob_refs_in(&md, &mut set);
        assert!(set.contains(&sha));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn export_names_links_and_prune() {
        let mut used = HashSet::new();
        assert_eq!(unique_file_name("a.pdf", &mut used), "a.pdf");
        assert_eq!(unique_file_name("a.pdf", &mut used), "a (2).pdf");
        assert_eq!(unique_file_name("a.pdf", &mut used), "a (3).pdf");
        assert_eq!(unique_file_name("README", &mut used), "README");
        assert_eq!(safe_file_name("../x/y\u{7}.txt"), "_x_y_.txt");
        assert_eq!(safe_file_name("  "), "file");

        let dir = std::env::temp_dir().join(format!("synthos-export-{}", project::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.bin");
        std::fs::write(&src, b"data").unwrap();
        let dst = dir.join("имя файла.bin");
        assert!(link_readonly(&src, &dst));
        assert!(same_file(&src, &dst), "жёсткая ссылка на кэш");
        assert!(std::fs::metadata(&dst).unwrap().permissions().readonly());
        assert!(link_readonly(&src, &dst), "повтор — без ошибок");

        // Выгрузка старше недели удаляется, свежая остаётся.
        let root = dir.join("export");
        for (name, days) in [("old", 8u64), ("fresh", 1)] {
            let d = root.join(name);
            std::fs::create_dir_all(&d).unwrap();
            let stamp = std::fs::File::create(d.join(EXPORT_STAMP)).unwrap();
            let age = std::time::Duration::from_secs(days * 24 * 3600);
            stamp.set_modified(std::time::SystemTime::now() - age).unwrap();
        }
        prune_exports(&root);
        assert!(!root.join("old").exists());
        assert!(root.join("fresh").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
