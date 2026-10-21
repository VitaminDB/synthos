//! Датасет: чтение и запись JSONL, редактор примеров.
//!
//! Файл — в `<библиотека>/datasets/`, примеры в любом виде, который понимает
//! обучение synaptix (`messages`, `conversations`, Alpaca, `prompt`/
//! `completion`, `text`). Редактор приводит их к диалогам; при сохранении
//! диалог пишется как `{"messages": [...]}`, сырой текст — как `{"text": …}`.
//!
//! Текст правится прямо в общей копии примеров (`Arc<Mutex<…>>`), без
//! пересборки виджетов: иначе каждое нажатие клавиши пересоздавало бы поле
//! и сбрасывало курсор. Вид перестраивается только при смене структуры —
//! добавили или убрали пример, реплику, перешли на другую страницу.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::{Dropdown, DropdownItem, MultilineTextEdit};

use crate::icons::*;

use super::state::{Selection, TrainingCtx};

/// Реплика диалога.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

/// Пример датасета.
#[derive(Clone, Debug, PartialEq)]
pub enum Example {
    Chat(Vec<Turn>),
    Text(String),
}

impl Example {
    fn parse(v: &Value) -> Option<Self> {
        let s = |v: &Value, keys: &[&str]| keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str)).map(str::to_string);
        if let Some(arr) = v.get("messages").or_else(|| v.get("conversations")).and_then(Value::as_array) {
            let turns = arr
                .iter()
                .map(|m| {
                    let role = match s(m, &["role", "from"]).unwrap_or_default().as_str() {
                        "system" => "system",
                        "assistant" | "gpt" | "bot" | "model" | "chatgpt" => "assistant",
                        _ => "user",
                    };
                    Turn { role: role.into(), content: s(m, &["content", "value", "text"]).unwrap_or_default() }
                })
                .collect();
            return Some(Example::Chat(turns));
        }
        if let Some(out) = s(v, &["output", "completion", "response", "answer", "chosen"]) {
            let mut prompt = s(v, &["instruction", "prompt", "question", "query"]).unwrap_or_default();
            if let Some(inp) = s(v, &["input", "context"]).filter(|x| !x.trim().is_empty()) {
                prompt = if prompt.is_empty() { inp } else { format!("{prompt}\n\n{inp}") };
            }
            let mut turns = Vec::new();
            if let Some(sys) = s(v, &["system"]).filter(|x| !x.is_empty()) {
                turns.push(Turn { role: "system".into(), content: sys });
            }
            turns.push(Turn { role: "user".into(), content: prompt });
            turns.push(Turn { role: "assistant".into(), content: out });
            return Some(Example::Chat(turns));
        }
        s(v, &["text", "content"]).map(Example::Text)
    }

    fn to_json(&self) -> Value {
        match self {
            Example::Chat(t) => json!({ "messages": t.iter().map(|m| json!({"role": m.role, "content": m.content})).collect::<Vec<_>>() }),
            Example::Text(t) => json!({ "text": t }),
        }
    }
}

/// Открытый датасет.
#[derive(Clone)]
pub struct DatasetDoc {
    pub path: PathBuf,
    pub examples: Arc<Mutex<Vec<Example>>>,
    /// Строки, которые не разобрались (пишутся обратно как есть).
    pub broken: Arc<Vec<String>>,
    /// Структура изменилась (пересобрать вид).
    pub rev: RwSignal<u64>,
    pub dirty: RwSignal<bool>,
    pub page: RwSignal<usize>,
}

impl PartialEq for DatasetDoc {
    fn eq(&self, o: &Self) -> bool {
        self.path == o.path && Arc::ptr_eq(&self.examples, &o.examples)
    }
}

impl std::fmt::Debug for DatasetDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DatasetDoc({})", self.path.display())
    }
}

pub const PAGE: usize = 20;

impl DatasetDoc {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut examples = Vec::new();
        let mut broken = Vec::new();
        let trimmed = text.trim_start();
        if trimmed.starts_with('[') {
            let v: Value = serde_json::from_str(trimmed).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            for x in v.as_array().into_iter().flatten() {
                match Example::parse(x) {
                    Some(e) => examples.push(e),
                    None => broken.push(x.to_string()),
                }
            }
        } else {
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                match serde_json::from_str::<Value>(line).ok().and_then(|v| Example::parse(&v)) {
                    Some(e) => examples.push(e),
                    None => broken.push(line.to_string()),
                }
            }
        }
        Ok(Self::with(path.to_path_buf(), examples, broken))
    }

    pub fn with(path: PathBuf, examples: Vec<Example>, broken: Vec<String>) -> Self {
        Self {
            path,
            examples: Arc::new(Mutex::new(examples)),
            broken: Arc::new(broken),
            rev: use_signal(0),
            dirty: use_signal(false),
            page: use_signal(0),
        }
    }

    pub fn len(&self) -> usize {
        self.examples.lock().unwrap().len()
    }

    /// Записать JSONL (через временный файл).
    pub fn save(&self) -> std::io::Result<()> {
        let mut out = String::new();
        for e in self.examples.lock().unwrap().iter() {
            out.push_str(&e.to_json().to_string());
            out.push('\n');
        }
        for b in self.broken.iter() {
            out.push_str(b);
            out.push('\n');
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, &self.path)
    }

    fn bump(&self) {
        self.rev.set(self.rev.get_untracked() + 1);
        self.dirty.set(true);
    }

    fn edit(&self, f: impl FnOnce(&mut Vec<Example>)) {
        f(&mut self.examples.lock().unwrap());
        self.bump();
    }
}

/// Открыть датасет в редакторе.
pub fn open(ctx: TrainingCtx, path: PathBuf) {
    if ctx.doc.get_untracked().is_some_and(|d| d.path == path) {
        ctx.selected.set(Selection::Dataset(path));
        return;
    }
    match DatasetDoc::load(&path) {
        Ok(d) => {
            ctx.doc.set(Some(d));
            ctx.selected.set(Selection::Dataset(path));
        }
        Err(e) => ctx.error.set(Some(tr!("training.dataset.error.read", path = crate::paths::pretty(&path), error = e.to_string()))),
    }
}

/// Создать пустой датасет в библиотеке и открыть его.
pub fn create(ctx: TrainingCtx, name: &str) {
    let Some(lib) = ctx.libraries.get_untracked().into_iter().next() else {
        ctx.error.set(Some(tr!("training.dataset.error.no_library")));
        return;
    };
    let clean: String = name.trim().chars().map(|c| if c == '/' || c == '\\' { '-' } else { c }).collect();
    if clean.is_empty() {
        return;
    }
    let path = lib.join("datasets").join(format!("{clean}.jsonl"));
    if path.exists() {
        open(ctx, path);
        return;
    }
    let doc = DatasetDoc::with(
        path.clone(),
        vec![Example::Chat(vec![Turn { role: "user".into(), content: String::new() }, Turn { role: "assistant".into(), content: String::new() }])],
        Vec::new(),
    );
    match doc.save() {
        Ok(()) => {
            ctx.doc.set(Some(doc));
            ctx.selected.set(Selection::Dataset(path));
            super::state::refresh(ctx);
        }
        Err(e) => ctx.error.set(Some(tr!("training.dataset.error.write", path = crate::paths::pretty(&path), error = e.to_string()))),
    }
}

fn save(ctx: TrainingCtx, doc: &DatasetDoc) {
    match doc.save() {
        Ok(()) => {
            doc.dirty.set(false);
            super::state::refresh(ctx);
        }
        Err(e) => ctx.error.set(Some(tr!("training.dataset.error.write", path = crate::paths::pretty(&doc.path), error = e.to_string()))),
    }
}

/// Шапка редактора: имя, число примеров, сохранить, добавить, обучить.
pub fn header_actions(ctx: TrainingCtx, doc: DatasetDoc) -> impl Widget {
    let d1 = doc.clone();
    let d2 = doc.clone();
    let d3 = doc.clone();
    Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let dirty = d3.dirty.get();
            let mut b = Button::new(tr!("training.dataset.save")).icon(MI_SAVE).on_click({
                let d = d3.clone();
                move || save(ctx, &d)
            });
            b = b.disabled(!dirty);
            vec![Box::new(b.class(if dirty { "training-btn training-btn-primary" } else { "training-btn" }))]
        }))
        .child(
            Button::new(tr!("training.dataset.add_example"))
                .icon(MI_ADD)
                .on_click(move || {
                    d1.edit(|v| v.push(Example::Chat(vec![Turn { role: "user".into(), content: String::new() }, Turn { role: "assistant".into(), content: String::new() }])));
                    let last = d1.len().saturating_sub(1) / PAGE;
                    d1.page.set(last);
                })
                .class("training-btn"),
        )
        .child(
            Button::new(tr!("training.dataset.train"))
                .icon(MI_MODEL_TRAINING)
                .on_click(move || {
                    if d2.dirty.get_untracked() {
                        save(ctx, &d2);
                    }
                    let mut f = ctx.form.get_untracked();
                    f.dataset = Some(d2.path.clone());
                    if let Some(m) = &f.model {
                        f.name = super::state::suggest_name(m, f.dataset.as_deref());
                    }
                    ctx.form.set(f);
                    ctx.check.set(super::state::Check::None);
                    ctx.selected.set(Selection::NewJob);
                })
                .class("training-btn"),
        )
}

const ROLES: [(&str, &str); 3] = [("system", "training.role.system"), ("user", "training.role.user"), ("assistant", "training.role.assistant")];

/// Тело редактора: страница примеров и переход между страницами.
pub fn view(ctx: TrainingCtx, doc: DatasetDoc) -> impl Widget {
    let d = doc.clone();
    let body = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        d.rev.get();
        let page = d.page.get();
        let total = d.len();
        let pages = total.div_ceil(PAGE).max(1);
        let page = page.min(pages - 1);
        let from = page * PAGE;
        let to = (from + PAGE).min(total);
        let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if !d.broken.is_empty() {
            col = col.child(Text::new(tr!("training.dataset.broken", n = d.broken.len())).class("training-warning"));
        }
        if total == 0 {
            col = col.child(Text::new(tr!("training.dataset.empty")).class("training-hint"));
        }
        for i in from..to {
            col = col.child(example_card(d.clone(), i));
        }
        col = col.child(pager(d.clone(), page, pages, total));
        vec![Box::new(col)]
    });
    let _ = ctx;
    ScrollView::new().vertical().child(DecoratedBox::new().class("training-center-body").child(body))
}

fn pager(doc: DatasetDoc, page: usize, pages: usize, total: usize) -> Box<dyn Widget> {
    let (p1, p2) = (doc.clone(), doc);
    Box::new(mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).class("training-pager") => [
            Button::new(tr!("training.dataset.prev")).disabled(page == 0).on_click(move || p1.page.set(page.saturating_sub(1))).class("training-btn"),
            Text::new(tr!("training.dataset.page", page = page + 1, pages = pages, total = total)).class("training-hint"),
            Button::new(tr!("training.dataset.next")).disabled(page + 1 >= pages).on_click(move || p2.page.set(page + 1)).class("training-btn"),
        ]
    })
}

fn example_card(doc: DatasetDoc, i: usize) -> Box<dyn Widget> {
    let ex = doc.examples.lock().unwrap().get(i).cloned();
    let Some(ex) = ex else { return Box::new(DecoratedBox::new()) };
    let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    let del = doc.clone();
    let head = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new(tr!("training.dataset.example", n = i + 1)).class("training-example-title"),
            DecoratedBox::new().class("grow"),
            ToolButton::new(MI_DELETE).tooltip(tr!("training.dataset.delete_example")).on_click(move || {
                del.edit(|v| { if i < v.len() { v.remove(i); } });
            }).class("training-icon-btn"),
        ]
    };
    col = col.child(head);
    match ex {
        Example::Text(t) => {
            let d = doc.clone();
            col = col.child(
                MultilineTextEdit::new()
                    .text(t)
                    .soft_wrap(true)
                    .auto_height(true)
                    .max_rows(16)
                    .on_change(move |s: &str| {
                        if let Some(Example::Text(x)) = d.examples.lock().unwrap().get_mut(i) {
                            *x = s.to_string();
                        }
                        d.dirty.set(true);
                    })
                    .class("training-turn-edit"),
            );
        }
        Example::Chat(turns) => {
            for (k, t) in turns.iter().enumerate() {
                col = col.child(turn_row(doc.clone(), i, k, t));
            }
            let (a1, a2) = (doc.clone(), doc.clone());
            col = col.child(mgui! {
                Row::new().gap(8.0) => [
                    Button::new(tr!("training.dataset.add_user")).icon(MI_ADD).on_click(move || {
                        a1.edit(|v| if let Some(Example::Chat(t)) = v.get_mut(i) { t.push(Turn { role: "user".into(), content: String::new() }) });
                    }).class("training-btn training-btn-small"),
                    Button::new(tr!("training.dataset.add_assistant")).icon(MI_ADD).on_click(move || {
                        a2.edit(|v| if let Some(Example::Chat(t)) = v.get_mut(i) { t.push(Turn { role: "assistant".into(), content: String::new() }) });
                    }).class("training-btn training-btn-small"),
                ]
            });
        }
    }
    Box::new(DecoratedBox::new().class("training-card").child(col))
}

fn turn_row(doc: DatasetDoc, i: usize, k: usize, t: &Turn) -> impl Widget {
    let mut dd = Dropdown::new().width(150.0);
    for (id, key) in ROLES {
        dd = dd.item(DropdownItem::new(id, tr!(key)));
    }
    let d1 = doc.clone();
    let dd = dd.selected(t.role.clone()).on_change(move |v: &str| {
        let v = v.to_string();
        d1.edit(|ex| {
            if let Some(Example::Chat(turns)) = ex.get_mut(i) {
                if let Some(t) = turns.get_mut(k) {
                    t.role = v;
                }
            }
        });
    });
    let d2 = doc.clone();
    let edit = MultilineTextEdit::new()
        .text(t.content.clone())
        .placeholder(tr!(match t.role.as_str() {
            "assistant" => "training.dataset.placeholder.assistant",
            "system" => "training.dataset.placeholder.system",
            _ => "training.dataset.placeholder.user",
        }))
        .soft_wrap(true)
        .auto_height(true)
        .max_rows(14)
        .on_change(move |s: &str| {
            if let Some(Example::Chat(turns)) = d2.examples.lock().unwrap().get_mut(i) {
                if let Some(t) = turns.get_mut(k) {
                    t.content = s.to_string();
                }
            }
            d2.dirty.set(true);
        });
    let d3 = doc.clone();
    let role_class = format!("training-turn training-turn-{}", t.role);
    DecoratedBox::new().class(role_class).child(mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
            dd,
            DecoratedBox::new().class("grow").child(edit.class("training-turn-edit")),
            ToolButton::new(MI_CLOSE).tooltip(tr!("training.dataset.delete_turn")).on_click(move || {
                d3.edit(|ex| if let Some(Example::Chat(turns)) = ex.get_mut(i) { if k < turns.len() { turns.remove(k); } });
            }).class("training-icon-btn"),
        ]
    })
}
