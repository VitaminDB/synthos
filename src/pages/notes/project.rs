//! Проект заметок — один файл `.syn` (контейнер synaptix-bundle).
//!
//! Раньше vault был папкой с `.md`-файлами; теперь весь проект — единый
//! бандл, самодостаточный и переносимый (вложения тоже внутри). Раскладка
//! File-чанков внутри бандла:
//!
//! ```text
//! notes/tree.json               дерево страниц: id / title / icon / children
//! notes/pages/<id>.md           markdown страницы
//! notes/objects/<id>.kanban.json канбан-доска (блок `![[kanban:<id>]]` в странице)
//! notes/objects/<id>.gantt.json  диаграмма Ганта (блок `![[gantt:<id>]]`)
//! notes/objects/<id>.mindmap.json интеллект-карта (блок `![[mindmap:<id>]]`)
//! notes/objects/<id>.calendar.json виджет календаря (блок `![[calendar:<id>]]`)
//! notes/calendar.json           единое хранилище событий календаря
//! notes/assets/<sha256>.<ext>   вложения (`asset:<sha256>.<ext>` в md)
//! ```
//!
//! Ключ страницы — стабильный id (12 hex), а не путь: переименование и
//! перенос по дереву не трогают ни файлы, ни ссылки. Запись — через
//! [`BundleEditor`] (append-only журнал + fsync, ~10 мс на commit); файл
//! растёт с каждым коммитом, поэтому при открытии проект уплотняется
//! ([`compact_if_needed`]), когда мусора больше половины.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use synaptix_bundle::{Bundle, BundleBuilder, BundleEditor, FileTag};

pub const TREE_PATH: &str = "notes/tree.json";
pub const PAGES_DIR: &str = "notes/pages";
pub const OBJECTS_DIR: &str = "notes/objects";
/// Единое хранилище событий календаря проекта.
pub const CALENDAR_PATH: &str = "notes/calendar.json";
pub const ASSETS_DIR: &str = "notes/assets";
/// `BundleMeta.purpose` — по нему проект отличается от модельных бандлов.
pub const BUNDLE_PURPOSE: &str = "notes";
pub const BUNDLE_ARCH: &str = "synthos-notes";
pub const TREE_VERSION: u32 = 1;

// ─────────────────────────── Дерево ───────────────────────────

/// Настройки раскладки страницы (панель «Свойства»). Сами координаты
/// блоков живут в markdown страницы — здесь только режим, сетка и привязка.
///
/// Дефолт — **свободная раскладка с привязкой** (шаг 5 px) и сеткой из
/// точек: страница ведёт себя как холст, поток включается вручную.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageLayout {
    /// Свободная раскладка: блоки ставятся мышью, а не колонкой потока.
    #[serde(default = "default_true")]
    pub free: bool,
    /// Фон холста.
    #[serde(default)]
    pub grid: PageGrid,
    #[serde(default = "default_grid_step")]
    pub grid_step: f32,
    /// Привязка к сетке при переносе — по умолчанию включена.
    #[serde(default = "default_true")]
    pub snap: bool,
    #[serde(default = "default_snap_step")]
    pub snap_step: f32,
    /// Фон страницы `#rrggbb` / `#rrggbbaa`; пусто — как в теме.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bg: String,
}

/// Фон холста свободной раскладки.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageGrid {
    None,
    #[default]
    Dots,
    Lines,
    Cross,
}

impl PageGrid {
    pub const ALL: [PageGrid; 4] = [PageGrid::None, PageGrid::Dots, PageGrid::Lines, PageGrid::Cross];
}

fn default_grid_step() -> f32 {
    20.0
}
fn default_snap_step() -> f32 {
    5.0
}
fn default_true() -> bool {
    true
}

impl Default for PageLayout {
    fn default() -> Self {
        Self {
            free: true,
            grid: PageGrid::default(),
            grid_step: default_grid_step(),
            snap: true,
            snap_step: default_snap_step(),
            bg: String::new(),
        }
    }
}

impl PageLayout {
    pub fn is_default(&self) -> bool {
        *self == PageLayout::default()
    }
}

/// Узел дерева «Содержимое»: страница с детьми.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PageNode {
    pub id: String,
    pub title: String,
    /// Иконка страницы: эмодзи либо глиф Material-шрифта (PUA).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Раскладка страницы; дефолт в файл не пишется.
    #[serde(default, skip_serializing_if = "PageLayout::is_default")]
    pub layout: PageLayout,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<PageNode>,
}

impl PageNode {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            title: title.into(),
            icon: None,
            layout: PageLayout::default(),
            children: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectTree {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub roots: Vec<PageNode>,
}

/// Плоская строка дерева для рендера (DFS с учётом свёрнутых узлов).
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
    pub depth: usize,
    pub has_children: bool,
}

impl ProjectTree {
    pub fn new() -> Self {
        Self { version: TREE_VERSION, roots: Vec::new() }
    }

    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    pub fn find(&self, id: &str) -> Option<&PageNode> {
        fn walk<'a>(nodes: &'a [PageNode], id: &str) -> Option<&'a PageNode> {
            for n in nodes {
                if n.id == id {
                    return Some(n);
                }
                if let Some(f) = walk(&n.children, id) {
                    return Some(f);
                }
            }
            None
        }
        walk(&self.roots, id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut PageNode> {
        fn walk<'a>(nodes: &'a mut [PageNode], id: &str) -> Option<&'a mut PageNode> {
            for n in nodes.iter_mut() {
                if n.id == id {
                    return Some(n);
                }
                if let Some(f) = walk(&mut n.children, id) {
                    return Some(f);
                }
            }
            None
        }
        walk(&mut self.roots, id)
    }

    pub fn title_of(&self, id: &str) -> Option<String> {
        self.find(id).map(|n| n.title.clone())
    }

    /// Раскладка страницы (дефолт — если узла нет).
    pub fn layout_of(&self, id: &str) -> PageLayout {
        self.find(id).map(|n| n.layout.clone()).unwrap_or_default()
    }

    pub fn icon_of(&self, id: &str) -> Option<String> {
        self.find(id).and_then(|n| n.icon.clone())
    }

    /// Родитель узла (`None` — корень или узел не найден).
    pub fn parent_of(&self, id: &str) -> Option<String> {
        fn walk(nodes: &[PageNode], id: &str) -> Option<String> {
            for n in nodes {
                if n.children.iter().any(|c| c.id == id) {
                    return Some(n.id.clone());
                }
                if let Some(p) = walk(&n.children, id) {
                    return Some(p);
                }
            }
            None
        }
        walk(&self.roots, id)
    }

    /// Цепочка предков от корня до самого узла: (id, title).
    pub fn path_of(&self, id: &str) -> Vec<(String, String)> {
        fn walk(nodes: &[PageNode], id: &str, acc: &mut Vec<(String, String)>) -> bool {
            for n in nodes {
                acc.push((n.id.clone(), n.title.clone()));
                if n.id == id || walk(&n.children, id, acc) {
                    return true;
                }
                acc.pop();
            }
            false
        }
        let mut acc = Vec::new();
        walk(&self.roots, id, &mut acc);
        acc
    }

    /// Является ли `ancestor` предком `id` (или им самим).
    pub fn is_ancestor_or_self(&self, ancestor: &str, id: &str) -> bool {
        self.path_of(id).iter().any(|(pid, _)| pid == ancestor)
    }

    /// Вырезать узел с поддеревом.
    pub fn remove(&mut self, id: &str) -> Option<PageNode> {
        fn walk(nodes: &mut Vec<PageNode>, id: &str) -> Option<PageNode> {
            if let Some(i) = nodes.iter().position(|n| n.id == id) {
                return Some(nodes.remove(i));
            }
            for n in nodes.iter_mut() {
                if let Some(r) = walk(&mut n.children, id) {
                    return Some(r);
                }
            }
            None
        }
        walk(&mut self.roots, id)
    }

    /// Вставить узел к родителю (`None` — в корень) на позицию `index`
    /// (`None` — в конец). Возвращает false, если родитель не найден.
    pub fn insert(&mut self, parent: Option<&str>, index: Option<usize>, node: PageNode) -> bool {
        let list = match parent {
            None => &mut self.roots,
            Some(pid) => match self.find_mut(pid) {
                Some(p) => &mut p.children,
                None => return false,
            },
        };
        let at = index.unwrap_or(list.len()).min(list.len());
        list.insert(at, node);
        true
    }

    /// Индекс узла среди соседей.
    pub fn index_in_parent(&self, id: &str) -> Option<usize> {
        let siblings = match self.parent_of(id) {
            None => &self.roots,
            Some(pid) => &self.find(&pid)?.children,
        };
        siblings.iter().position(|n| n.id == id)
    }

    /// Плоский список для рендера; дети свёрнутых узлов пропускаются.
    pub fn flatten(&self, expanded: &HashSet<String>) -> Vec<TreeRow> {
        fn walk(nodes: &[PageNode], depth: usize, expanded: &HashSet<String>, out: &mut Vec<TreeRow>) {
            for n in nodes {
                out.push(TreeRow {
                    id: n.id.clone(),
                    title: n.title.clone(),
                    icon: n.icon.clone(),
                    depth,
                    has_children: !n.children.is_empty(),
                });
                if !n.children.is_empty() && expanded.contains(&n.id) {
                    walk(&n.children, depth + 1, expanded, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.roots, 0, expanded, &mut out);
        out
    }

    /// Все узлы (DFS) — для индекса и поиска.
    pub fn all(&self) -> Vec<&PageNode> {
        fn walk<'a>(nodes: &'a [PageNode], out: &mut Vec<&'a PageNode>) {
            for n in nodes {
                out.push(n);
                walk(&n.children, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.roots, &mut out);
        out
    }

    pub fn all_ids(&self) -> Vec<String> {
        self.all().into_iter().map(|n| n.id.clone()).collect()
    }

    /// Id узла и всех его потомков.
    pub fn subtree_ids(&self, id: &str) -> Vec<String> {
        fn walk(n: &PageNode, out: &mut Vec<String>) {
            out.push(n.id.clone());
            for c in &n.children {
                walk(c, out);
            }
        }
        let mut out = Vec::new();
        if let Some(n) = self.find(id) {
            walk(n, &mut out);
        }
        out
    }

    pub fn first_id(&self) -> Option<String> {
        self.roots.first().map(|n| n.id.clone())
    }

    /// Страница по названию (без регистра) — цель wiki-ссылки.
    pub fn resolve_title(&self, title: &str) -> Option<String> {
        let key = title.trim().to_lowercase();
        self.all()
            .into_iter()
            .find(|n| n.title.trim().to_lowercase() == key)
            .map(|n| n.id.clone())
    }
}

/// Новый стабильный id: 12 hex из случайного u64.
pub fn new_id() -> String {
    let v: u64 = rand::random();
    format!("{:012x}", v & 0xFFFF_FFFF_FFFF)
}

/// Глубокая копия поддерева с новыми id; возвращает пары (старый, новый).
pub fn clone_subtree(node: &PageNode, map: &mut Vec<(String, String)>) -> PageNode {
    let id = new_id();
    map.push((node.id.clone(), id.clone()));
    PageNode {
        id,
        title: node.title.clone(),
        icon: node.icon.clone(),
        layout: node.layout.clone(),
        children: node.children.iter().map(|c| clone_subtree(c, map)).collect(),
    }
}

// ─────────────────────────── Пути в бандле ───────────────────────────

pub fn page_path(id: &str) -> String {
    format!("{PAGES_DIR}/{id}.md")
}

/// `kind` — `kanban` | `gantt`.
pub fn object_path(kind: &str, id: &str) -> String {
    format!("{OBJECTS_DIR}/{id}.{kind}.json")
}

pub fn asset_path(name: &str) -> String {
    format!("{ASSETS_DIR}/{name}")
}

// ─────────────────────────── Файл проекта ───────────────────────────

/// Путь проекта: настройка либо дефолт `~/Documents/SynthOS Notes.syn`.
pub fn resolve_project_path(configured: &str) -> PathBuf {
    if !configured.trim().is_empty() {
        return PathBuf::from(expand_home(configured.trim()));
    }
    home_dir().join("Documents").join("SynthOS Notes.syn")
}

/// Папка старого vault'а (первая волна): источник миграции.
pub fn legacy_vault_path(configured: &str) -> PathBuf {
    if !configured.trim().is_empty() {
        return PathBuf::from(expand_home(configured.trim()));
    }
    home_dir().join("Documents").join("SynthOS Notes")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn expand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        return home_dir().join(rest).display().to_string();
    }
    p.to_string()
}

/// Имя проекта для плитки рейла — имя файла без расширения.
pub fn project_title(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Notes".to_string())
}

/// Операция записи в бандл (пачка применяется одним commit'ом).
#[derive(Clone, Debug)]
pub enum WriteOp {
    Put { path: String, bytes: Vec<u8> },
    Remove { path: String },
}

pub type ProjectResult<T> = Result<T, String>;

/// Создать пустой проект (или из готового набора файлов — миграция).
pub fn create(path: &Path, tree: &ProjectTree, files: Vec<(String, Vec<u8>)>) -> ProjectResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut builder = BundleBuilder::new(format!("notes-{}", new_id()), "1")
        .arch(BUNDLE_ARCH)
        .purpose(BUNDLE_PURPOSE)
        .add_file_bytes(TREE_PATH, tree.serialize().into_bytes(), FileTag::Doc)
        .map_err(|e| e.to_string())?;
    for (p, bytes) in files {
        builder = builder.add_file_bytes(&p, bytes, tag_for(&p)).map_err(|e| e.to_string())?;
    }
    builder.write(path).map_err(|e| e.to_string())
}

fn tag_for(path: &str) -> FileTag {
    if path.starts_with(ASSETS_DIR) {
        FileTag::Asset
    } else {
        FileTag::Doc
    }
}

/// Применить пачку операций одним commit'ом.
pub fn apply_ops(path: &Path, ops: &[WriteOp]) -> ProjectResult<()> {
    if ops.is_empty() {
        return Ok(());
    }
    let mut editor = BundleEditor::open(path).map_err(|e| e.to_string())?;
    for op in ops {
        match op {
            WriteOp::Put { path: p, bytes } => {
                editor
                    .replace_file(p, bytes.clone(), tag_for(p))
                    .map_err(|e| e.to_string())?;
            }
            WriteOp::Remove { path: p } => {
                // Удаление отсутствующего файла — не ошибка.
                let _ = editor.remove_file(p);
            }
        }
    }
    editor.commit().map_err(|e| e.to_string())?;
    invalidate_cache();
    Ok(())
}

/// Уплотнить файл, если журнал правок разросся: мёртвых байт больше
/// живых (и больше 1 МБ). Вызывается при открытии проекта.
pub fn compact_if_needed(path: &Path) {
    let Ok(bundle) = Bundle::open(path) else { return };
    let alive: u64 = bundle
        .cdir()
        .entries
        .iter()
        .filter(|e| e.is_alive())
        .map(|e| e.payload_len + e.header_len as u64)
        .sum();
    let size = bundle.size();
    drop(bundle);
    let dead = size.saturating_sub(alive);
    if dead > alive.max(1024 * 1024) {
        if let Err(e) = synaptix_bundle::compact(path, path) {
            log::warn!("notes: уплотнение {} не удалось: {e}", path.display());
        } else {
            log::info!("notes: проект уплотнён ({size} → живых {alive} байт)");
        }
        invalidate_cache();
    }
}

// ─────────────────────────── Чтение (кэш mmap) ───────────────────────────

fn cache() -> &'static Mutex<Option<(PathBuf, Arc<Bundle>)>> {
    static C: OnceLock<Mutex<Option<(PathBuf, Arc<Bundle>)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// Открытый на чтение бандл (mmap). Пересоздаётся после каждого commit'а —
/// append-only формат означает, что старая карта не видит новых чанков.
pub fn bundle(path: &Path) -> Option<Arc<Bundle>> {
    let mut guard = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, b)) = guard.as_ref() {
        if p == path {
            return Some(b.clone());
        }
    }
    match Bundle::open(path) {
        Ok(b) => {
            let arc = Arc::new(b);
            *guard = Some((path.to_path_buf(), arc.clone()));
            Some(arc)
        }
        Err(e) => {
            log::warn!("notes: не удалось открыть проект {}: {e}", path.display());
            None
        }
    }
}

pub fn invalidate_cache() {
    *cache().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn read_bytes(path: &Path, bundle_path: &str) -> Option<Vec<u8>> {
    let b = bundle(path)?;
    b.read_file(bundle_path).ok().map(|c| c.into_owned())
}

pub fn read_text(path: &Path, bundle_path: &str) -> Option<String> {
    read_bytes(path, bundle_path).and_then(|v| String::from_utf8(v).ok())
}

pub fn has_file(path: &Path, bundle_path: &str) -> bool {
    bundle(path)
        .map(|b| b.list_files().any(|e| e.name == bundle_path))
        .unwrap_or(false)
}

pub fn read_tree(path: &Path) -> ProjectTree {
    match read_text(path, TREE_PATH) {
        Some(json) => match ProjectTree::parse(&json) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("notes: дерево проекта не распарсилось: {e}");
                ProjectTree::new()
            }
        },
        None => ProjectTree::new(),
    }
}

/// Имена всех вложений проекта (для GC).
pub fn list_assets(path: &Path) -> Vec<String> {
    let Some(b) = bundle(path) else { return Vec::new() };
    let prefix = format!("{ASSETS_DIR}/");
    b.list_files()
        .filter_map(|e| e.name.strip_prefix(&prefix).map(|s| s.to_string()))
        .collect()
}

// ─────────────────────────── Миграция старого vault'а ───────────────────────────

/// Импорт папки первой волны (`.md`, вложенность папок) в дерево + файлы
/// бандла. Папка `X/` рядом с `X.md` становится детьми страницы X. Базы и
/// канвасы тех волн (`*.base.json`, `*.canvas.json`) больше не
/// поддерживаются и пропускаются. Пустая/отсутствующая папка → `None`.
pub fn migrate_folder(root: &Path) -> Option<(ProjectTree, Vec<(String, Vec<u8>)>)> {
    if !root.is_dir() {
        return None;
    }
    let mut files = Vec::new();
    let roots = migrate_dir(root, &mut files);
    if roots.is_empty() {
        return None;
    }
    Some((ProjectTree { version: TREE_VERSION, roots }, files))
}

fn migrate_dir(dir: &Path, files: &mut Vec<(String, Vec<u8>)>) -> Vec<PageNode> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<(String, bool)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            let is_dir = e.file_type().ok()?.is_dir();
            Some((name, is_dir))
        })
        .collect();
    names.sort_by_key(|(n, _)| n.to_lowercase());
    let dirs: HashSet<String> =
        names.iter().filter(|(_, d)| *d).map(|(n, _)| n.clone()).collect();
    let mut out = Vec::new();
    let mut consumed_dirs: HashSet<String> = HashSet::new();
    for (name, is_dir) in &names {
        if *is_dir {
            continue;
        }
        let lower = name.to_lowercase();
        if !lower.ends_with(".md") {
            continue;
        }
        let title = name[..name.len() - 3].to_string();
        let Ok(bytes) = std::fs::read(dir.join(name)) else { continue };
        let mut node = PageNode::new(title.clone());
        files.push((page_path(&node.id), bytes));
        if dirs.contains(&title) {
            node.children = migrate_dir(&dir.join(&title), files);
            consumed_dirs.insert(title);
        }
        out.push(node);
    }
    // Папки без страницы-спутника — страницы-контейнеры с пустым текстом.
    for (name, is_dir) in &names {
        if !*is_dir || consumed_dirs.contains(name) {
            continue;
        }
        let children = migrate_dir(&dir.join(name), files);
        if children.is_empty() {
            continue;
        }
        let mut node = PageNode::new(name.clone());
        node.children = children;
        files.push((page_path(&node.id), Vec::new()));
        out.push(node);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_layout_defaults_and_roundtrip() {
        // Привязка включена по умолчанию с шагом 5 px (запрос UX).
        let def = PageLayout::default();
        assert!(def.snap);
        assert_eq!(def.snap_step, 5.0);
        assert!(def.free, "свободная раскладка — режим по умолчанию");

        // Дефолт не попадает в tree.json — старые проекты читаются как есть.
        let tree = sample_tree();
        let json = tree.serialize();
        assert!(!json.contains("layout"), "дефолтная раскладка не должна писаться:\n{json}");
        assert_eq!(ProjectTree::parse(&json).unwrap().layout_of("a"), def);

        // Изменённая — сохраняется и читается обратно.
        let mut tree = sample_tree();
        tree.find_mut("b").unwrap().layout = PageLayout {
            free: false,
            grid: PageGrid::Lines,
            grid_step: 25.0,
            snap: false,
            snap_step: 2.0,
            bg: "#243149".to_string(),
        };
        let json = tree.serialize();
        assert!(json.contains("\"bg\": \"#243149\""), "{json}");
        let back = ProjectTree::parse(&json).unwrap();
        assert_eq!(back.layout_of("b"), tree.layout_of("b"));
        assert_eq!(back.layout_of("c"), PageLayout::default());
        // Пустой фон не пишется.
        tree.find_mut("b").unwrap().layout.bg.clear();
        assert!(!tree.serialize().contains("\"bg\""));
    }

    fn sample_tree() -> ProjectTree {
        let mut a = PageNode::new("A");
        a.id = "a".into();
        let mut b = PageNode::new("B");
        b.id = "b".into();
        let mut c = PageNode::new("C");
        c.id = "c".into();
        b.children.push(c);
        a.children.push(b);
        let mut d = PageNode::new("D");
        d.id = "d".into();
        ProjectTree { version: 1, roots: vec![a, d] }
    }

    #[test]
    fn tree_navigation() {
        let t = sample_tree();
        assert_eq!(t.parent_of("c").as_deref(), Some("b"));
        assert_eq!(t.parent_of("a"), None);
        assert_eq!(t.path_of("c").len(), 3);
        assert!(t.is_ancestor_or_self("a", "c"));
        assert!(!t.is_ancestor_or_self("d", "c"));
        assert_eq!(t.subtree_ids("a"), vec!["a", "b", "c"]);
        assert_eq!(t.index_in_parent("d"), Some(1));
        assert_eq!(t.resolve_title(" c "), Some("c".into()));
    }

    #[test]
    fn tree_flatten_respects_expanded() {
        let t = sample_tree();
        let rows = t.flatten(&HashSet::new());
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["a", "d"]);
        let mut exp = HashSet::new();
        exp.insert("a".to_string());
        let rows = t.flatten(&exp);
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["a", "b", "d"]);
        assert_eq!(rows[1].depth, 1);
        assert!(rows[1].has_children);
    }

    #[test]
    fn tree_move_and_remove() {
        let mut t = sample_tree();
        let b = t.remove("b").unwrap();
        assert!(t.find("c").is_none());
        assert!(t.insert(Some("d"), Some(0), b));
        assert_eq!(t.parent_of("c").as_deref(), Some("b"));
        assert_eq!(t.parent_of("b").as_deref(), Some("d"));
        assert!(!t.insert(Some("nope"), None, PageNode::new("x")));
    }

    #[test]
    fn tree_roundtrip_json() {
        let t = sample_tree();
        let json = t.serialize();
        assert_eq!(ProjectTree::parse(&json).unwrap(), t);
    }

    #[test]
    fn bundle_create_write_read() {
        let dir = std::env::temp_dir().join(format!("synthos-notes-test-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.syn");
        let tree = sample_tree();
        create(&path, &tree, vec![(page_path("a"), b"# A".to_vec())]).unwrap();
        assert_eq!(read_tree(&path), tree);
        assert_eq!(read_text(&path, &page_path("a")).as_deref(), Some("# A"));
        apply_ops(
            &path,
            &[
                WriteOp::Put { path: page_path("a"), bytes: b"# A2".to_vec() },
                WriteOp::Put { path: page_path("d"), bytes: b"D".to_vec() },
            ],
        )
        .unwrap();
        assert_eq!(read_text(&path, &page_path("a")).as_deref(), Some("# A2"));
        assert!(has_file(&path, &page_path("d")));
        apply_ops(&path, &[WriteOp::Remove { path: page_path("d") }]).unwrap();
        assert!(!has_file(&path, &page_path("d")));
        // Удаление несуществующего — не ошибка.
        apply_ops(&path, &[WriteOp::Remove { path: page_path("zz") }]).unwrap();
        compact_if_needed(&path);
        assert_eq!(read_text(&path, &page_path("a")).as_deref(), Some("# A2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_folder_nests_folder_notes() {
        let dir = std::env::temp_dir().join(format!("synthos-notes-mig-{}", new_id()));
        std::fs::create_dir_all(dir.join("Проект")).unwrap();
        std::fs::write(dir.join("Проект.md"), "root").unwrap();
        std::fs::write(dir.join("Проект/Идеи.md"), "ideas").unwrap();
        // Базы первой волны больше не импортируются.
        std::fs::write(dir.join("План.base.json"), "{}").unwrap();
        let (tree, files) = migrate_folder(&dir).unwrap();
        assert_eq!(tree.roots.len(), 1);
        let proj = tree.roots.iter().find(|n| n.title == "Проект").unwrap();
        assert_eq!(proj.children[0].title, "Идеи");
        assert!(tree.roots.iter().all(|n| n.title != "План"));
        assert!(!files.iter().any(|(p, _)| p.starts_with(OBJECTS_DIR)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
