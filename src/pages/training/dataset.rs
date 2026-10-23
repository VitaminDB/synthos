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
use syngui::widgets::{MultilineTextEdit, TextField};

use crate::icons::*;

use super::state::{Selection, TrainingCtx};

/// Вызов инструмента в ответе ассистента: имя функции и аргументы (JSON-текст).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ToolCall {
    pub name: String,
    pub args: String,
}

/// Реплика диалога. `name` — имя функции у результата инструмента
/// (`role = "tool"`), `calls` — вызовы ассистента; прочие поля сообщения
/// (`reasoning_content`, `tool_call_id`, …) хранятся в `extra` и пишутся обратно.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Turn {
    pub role: String,
    pub content: String,
    pub name: String,
    pub calls: Vec<ToolCall>,
    pub extra: serde_json::Map<String, Value>,
}

impl Turn {
    pub fn new(role: &str) -> Self {
        Self { role: role.into(), ..Default::default() }
    }
}

/// Пример датасета. У диалога `tools` — описания функций (JSON-текст
/// массива в формате OpenAI, пусто — без инструментов), `extra` — прочие
/// поля строки.
#[derive(Clone, Debug, PartialEq)]
pub enum Example {
    Chat { turns: Vec<Turn>, tools: String, extra: serde_json::Map<String, Value> },
    Text(String),
}

fn call_of(v: &Value) -> Option<ToolCall> {
    let f = v.get("function").unwrap_or(v);
    let name = f.get("name")?.as_str()?.to_string();
    let args = match f.get("arguments").or_else(|| f.get("parameters")) {
        Some(Value::String(s)) => pretty_json(s),
        Some(x) => serde_json::to_string_pretty(x).unwrap_or_default(),
        None => "{}".into(),
    };
    Some(ToolCall { name, args })
}

/// JSON-текст в читаемом виде (не JSON — как есть).
fn pretty_json(s: &str) -> String {
    serde_json::from_str::<Value>(s).ok().and_then(|v| serde_json::to_string_pretty(&v).ok()).unwrap_or_else(|| s.to_string())
}

/// Значение JSON из текста поля: разобранное или (не JSON) строка как есть.
fn json_or_string(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.to_string()))
}

impl Example {
    pub fn new_chat() -> Self {
        Example::Chat { turns: vec![Turn::new("user"), Turn::new("assistant")], tools: String::new(), extra: Default::default() }
    }

    fn parse(v: &Value) -> Option<Self> {
        let s = |v: &Value, keys: &[&str]| keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str)).map(str::to_string);
        if let Some(arr) = v.get("messages").or_else(|| v.get("conversations")).and_then(Value::as_array) {
            let mut turns: Vec<Turn> = Vec::new();
            for m in arr {
                let raw = s(m, &["role", "from"]).unwrap_or_default();
                let content = s(m, &["content", "value", "text"]).unwrap_or_default();
                // ShareGPT: вызов — отдельная реплика; к ответу ассистента перед ней.
                if raw == "function_call" {
                    if let Some(c) = serde_json::from_str::<Value>(&content).ok().as_ref().and_then(call_of) {
                        match turns.last_mut() {
                            Some(t) if t.role == "assistant" => t.calls.push(c),
                            _ => turns.push(Turn { calls: vec![c], ..Turn::new("assistant") }),
                        }
                    }
                    continue;
                }
                let role = match raw.as_str() {
                    "system" => "system",
                    "assistant" | "gpt" | "bot" | "model" | "chatgpt" => "assistant",
                    "tool" | "function" | "observation" | "ipython" => "tool",
                    _ => "user",
                };
                let mut t = Turn { content, ..Turn::new(role) };
                if let Some(o) = m.as_object() {
                    for (k, x) in o {
                        match k.as_str() {
                            "role" | "from" | "content" | "value" | "text" => {}
                            "name" if role == "tool" => t.name = x.as_str().unwrap_or_default().to_string(),
                            "tool_calls" if role == "assistant" => t.calls.extend(x.as_array().into_iter().flatten().filter_map(call_of)),
                            "function_call" if role == "assistant" => t.calls.extend(call_of(x)),
                            _ => {
                                t.extra.insert(k.clone(), x.clone());
                            }
                        }
                    }
                }
                if role == "tool" && t.name.is_empty() {
                    t.name = turns.iter().rev().find(|x| x.role == "assistant").and_then(|a| a.calls.last()).map(|c| c.name.clone()).unwrap_or_default();
                }
                turns.push(t);
            }
            let tools = match v.get("tools") {
                Some(Value::String(x)) => pretty_json(x),
                Some(x @ Value::Array(a)) if !a.is_empty() => serde_json::to_string_pretty(x).unwrap_or_default(),
                _ => String::new(),
            };
            let mut extra = v.as_object().cloned().unwrap_or_default();
            for k in ["messages", "conversations", "tools"] {
                extra.remove(k);
            }
            return Some(Example::Chat { turns, tools, extra });
        }
        if let Some(out) = s(v, &["output", "completion", "response", "answer", "chosen"]) {
            let mut prompt = s(v, &["instruction", "prompt", "question", "query"]).unwrap_or_default();
            if let Some(inp) = s(v, &["input", "context"]).filter(|x| !x.trim().is_empty()) {
                prompt = if prompt.is_empty() { inp } else { format!("{prompt}\n\n{inp}") };
            }
            let mut turns = Vec::new();
            if let Some(sys) = s(v, &["system"]).filter(|x| !x.is_empty()) {
                turns.push(Turn { content: sys, ..Turn::new("system") });
            }
            turns.push(Turn { content: prompt, ..Turn::new("user") });
            turns.push(Turn { content: out, ..Turn::new("assistant") });
            return Some(Example::Chat { turns, tools: String::new(), extra: Default::default() });
        }
        s(v, &["text", "content"]).map(Example::Text)
    }

    /// Строка JSONL в формате OpenAI (`messages`, `tools`, `tool_calls`).
    fn to_json(&self) -> Value {
        match self {
            Example::Chat { turns, tools, extra } => {
                let msgs: Vec<Value> = turns
                    .iter()
                    .map(|t| {
                        let mut m = t.extra.clone();
                        m.insert("role".into(), json!(t.role));
                        m.insert("content".into(), json!(t.content));
                        if t.role == "tool" && !t.name.trim().is_empty() {
                            m.insert("name".into(), json!(t.name.trim()));
                        }
                        if t.role == "assistant" && !t.calls.is_empty() {
                            let calls: Vec<Value> = t
                                .calls
                                .iter()
                                .map(|c| {
                                    // аргументы — строкой компактного JSON, как у OpenAI
                                    let args = serde_json::from_str::<Value>(&c.args).map(|v| v.to_string()).unwrap_or_else(|_| c.args.clone());
                                    json!({"type": "function", "function": {"name": c.name.trim(), "arguments": args}})
                                })
                                .collect();
                            m.insert("tool_calls".into(), Value::Array(calls));
                        }
                        Value::Object(m)
                    })
                    .collect();
                let mut o = extra.clone();
                o.insert("messages".into(), Value::Array(msgs));
                if !tools.trim().is_empty() {
                    o.insert("tools".into(), json_or_string(tools.trim()));
                }
                Value::Object(o)
            }
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
        vec![Example::new_chat()],
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
                    d1.edit(|v| v.push(Example::new_chat()));
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

/// Роли по кругу: щелчок по метке роли переключает на следующую.
const ROLES: [&str; 4] = ["user", "assistant", "tool", "system"];

fn role_label(role: &str) -> String {
    tr!(match role {
        "assistant" => "training.role.assistant",
        "system" => "training.role.system",
        "tool" => "training.role.tool",
        _ => "training.role.user",
    })
}

fn role_icon(role: &str) -> &'static str {
    match role {
        "assistant" => MI_AUTO_AWESOME,
        "system" => MI_TUNE,
        "tool" => MI_DATA_OBJECT,
        _ => MI_PERSON,
    }
}

fn next_role(role: &str) -> String {
    let i = ROLES.iter().position(|r| *r == role).unwrap_or(0);
    ROLES[(i + 1) % ROLES.len()].to_string()
}

/// Заготовка описаний инструментов (формат OpenAI).
const TOOLS_TEMPLATE: &str = "[\n  {\n    \"type\": \"function\",\n    \"function\": {\n      \"name\": \"\",\n      \"description\": \"\",\n      \"parameters\": {\"type\": \"object\", \"properties\": {}, \"required\": []}\n    }\n  }\n]";

/// Правка текста реплики без пересборки вида (курсор не сбрасывается).
fn with_turn(doc: &DatasetDoc, i: usize, k: usize, f: impl FnOnce(&mut Turn)) {
    if let Some(Example::Chat { turns, .. }) = doc.examples.lock().unwrap().get_mut(i) {
        if let Some(t) = turns.get_mut(k) {
            f(t);
        }
    }
    doc.dirty.set(true);
}

/// Правка структуры диалога (с пересборкой вида).
fn edit_chat(doc: &DatasetDoc, i: usize, f: impl FnOnce(&mut Vec<Turn>, &mut String)) {
    doc.edit(|v| {
        if let Some(Example::Chat { turns, tools, .. }) = v.get_mut(i) {
            f(turns, tools);
        }
    });
}

fn tools_count(tools: &str) -> Option<usize> {
    serde_json::from_str::<Value>(tools).ok().and_then(|v| v.as_array().map(Vec::len))
}

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
        let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if !d.broken.is_empty() {
            col = col.child(Text::new(tr!("training.dataset.broken", n = d.broken.len())).class("training-warning"));
        }
        if total == 0 {
            col = col.child(Text::new(tr!("training.dataset.empty")).class("training-hint"));
        }
        for i in from..to {
            col = col.child(example_card(d.clone(), i));
        }
        if pages > 1 {
            col = col.child(pager(d.clone(), page, pages, total));
        }
        vec![Box::new(col)]
    });
    let _ = ctx;
    ScrollView::new().vertical().child(DecoratedBox::new().class("dsx-body").child(body))
}

fn pager(doc: DatasetDoc, page: usize, pages: usize, total: usize) -> Box<dyn Widget> {
    let (p1, p2) = (doc.clone(), doc);
    Box::new(mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).class("dsx-pager") => [
            Button::new(tr!("training.dataset.prev")).disabled(page == 0).on_click(move || p1.page.set(page.saturating_sub(1))).class("dsx-link"),
            Text::new(tr!("training.dataset.page", page = page + 1, pages = pages, total = total)).class("dsx-muted"),
            Button::new(tr!("training.dataset.next")).disabled(page + 1 >= pages).on_click(move || p2.page.set(page + 1)).class("dsx-link"),
        ]
    })
}

fn pill(text: String, class: &str) -> impl Widget {
    DecoratedBox::new().class(format!("dsx-pill {class}")).child(Text::new(text).class("dsx-pill-text"))
}

fn icon_btn(icon: &str, tip: String, on: impl FnMut() + Send + 'static) -> impl Widget {
    ToolButton::new(icon).tooltip(tip).on_click(on).class("dsx-icon")
}

/// Поле текста без своей рамки (рамку и фон снимает `.dsx-text MultilineTextEdit`).
fn text_area(text: String, placeholder: String, mono: bool, max_rows: usize, on: impl FnMut(&str) + Send + 'static) -> impl Widget {
    DecoratedBox::new().class(if mono { "dsx-text dsx-mono" } else { "dsx-text" }).child(
        MultilineTextEdit::new().text(text).placeholder(placeholder).rows(1).soft_wrap(true).auto_height(true).max_rows(max_rows).on_change(on),
    )
}

fn example_card(doc: DatasetDoc, i: usize) -> Box<dyn Widget> {
    let ex = doc.examples.lock().unwrap().get(i).cloned();
    let Some(ex) = ex else { return Box::new(DecoratedBox::new()) };

    // Шапка: номер, сводка, действия.
    let mut head = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    head = head.child(Text::new(format!("#{}", i + 1)).class("dsx-num"));
    if let Example::Chat { turns, tools, .. } = &ex {
        head = head.child(pill(tr!("training.dataset.turns", n = turns.len()), ""));
        let calls: usize = turns.iter().map(|t| t.calls.len()).sum();
        if calls > 0 {
            head = head.child(pill(tr!("training.dataset.calls", n = calls), "dsx-pill-tool"));
        }
        if !tools.trim().is_empty() {
            let n = tools_count(tools).map(|n| n.to_string()).unwrap_or_else(|| "?".into());
            head = head.child(pill(tr!("training.dataset.tools_count", n = n), "dsx-pill-tool"));
        }
    }
    head = head.child(DecoratedBox::new().class("grow"));
    if let Example::Chat { tools, .. } = &ex {
        if tools.trim().is_empty() {
            let d = doc.clone();
            head = head.child(icon_btn(MI_HANDYMAN, tr!("training.dataset.tools_add"), move || edit_chat(&d, i, |_, tools| *tools = TOOLS_TEMPLATE.into())));
        }
    }
    let del = doc.clone();
    head = head.child(icon_btn(MI_DELETE, tr!("training.dataset.delete_example"), move || {
        del.edit(|v| {
            if i < v.len() {
                v.remove(i);
            }
        })
    }));

    let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head);
    match ex {
        Example::Text(t) => {
            let d = doc.clone();
            col = col.child(text_area(t, String::new(), false, 24, move |s: &str| {
                if let Some(Example::Text(x)) = d.examples.lock().unwrap().get_mut(i) {
                    *x = s.to_string();
                }
                d.dirty.set(true);
            }));
        }
        Example::Chat { turns, tools, .. } => {
            if !tools.trim().is_empty() {
                col = col.child(tools_panel(doc.clone(), i, tools));
            }
            for (k, t) in turns.iter().enumerate() {
                col = col.child(turn_view(doc.clone(), i, k, t));
            }
            let link = |role: &'static str, key: &str| {
                let d = doc.clone();
                Button::new(tr!(key)).on_click(move || edit_chat(&d, i, |t, _| t.push(Turn::new(role)))).class("dsx-link")
            };
            col = col.child(mgui! {
                Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).class("dsx-add-row") => [
                    Text::new(MI_ADD).class("dsx-add-icon"),
                    link("user", "training.dataset.add_user"),
                    link("assistant", "training.dataset.add_assistant"),
                    link("tool", "training.dataset.add_tool_result"),
                    link("system", "training.dataset.add_system"),
                ]
            });
        }
    }
    Box::new(DecoratedBox::new().class("dsx-card").child(col))
}

/// Описания инструментов примера: JSON-массив в формате OpenAI.
fn tools_panel(doc: DatasetDoc, i: usize, tools: String) -> impl Widget {
    let valid = use_signal(tools_count(&tools).is_some());
    let d = doc.clone();
    let edit = text_area(tools, String::new(), true, 22, move |s: &str| {
        if let Some(Example::Chat { tools, .. }) = d.examples.lock().unwrap().get_mut(i) {
            *tools = s.to_string();
        }
        valid.set(tools_count(s).is_some() || s.trim().is_empty());
        d.dirty.set(true);
    });
    let rm = doc.clone();
    DecoratedBox::new().class("dsx-tools").child(mgui! {
        Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Text::new(MI_HANDYMAN).class("dsx-glyph dsx-glyph-tools"),
                Text::new(tr!("training.dataset.tools")).class("dsx-role dsx-role-tools"),
                Text::new(tr!("training.dataset.tools_hint")).max_lines(2).class("dsx-muted grow"),
                icon_btn(MI_CLOSE, tr!("training.dataset.tools_remove"), move || edit_chat(&rm, i, |_, tools| tools.clear())),
            ],
            edit,
            Reactive::new(move || -> Vec<Box<dyn Widget>> {
                if valid.get() { vec![] } else { vec![Box::new(Text::new(tr!("training.dataset.tools_invalid")).class("training-warning"))] }
            }),
        ]
    })
}

fn turn_view(doc: DatasetDoc, i: usize, k: usize, t: &Turn) -> impl Widget {
    let role = t.role.clone();
    // Метка роли: щелчок — следующая роль.
    let d1 = doc.clone();
    let mut head = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(
        Button::new(role_label(&role))
            .on_click(move || {
                edit_chat(&d1, i, |turns, _| {
                    if let Some(t) = turns.get_mut(k) {
                        t.role = next_role(&t.role);
                    }
                })
            })
            .class(&format!("dsx-role dsx-role-{role}")),
    );
    if role == "tool" {
        let d = doc.clone();
        head = head.child(DecoratedBox::new().class("dsx-name").child(
            TextField::new().text(t.name.clone()).placeholder(tr!("training.dataset.placeholder.tool_name")).width(200.0).on_change(move |s: &str| {
                let s = s.to_string();
                with_turn(&d, i, k, |t| t.name = s);
            }),
        ));
    }
    head = head.child(DecoratedBox::new().class("grow"));
    if role == "assistant" {
        let d = doc.clone();
        head = head.child(
            Button::new(tr!("training.dataset.add_call"))
                .on_click(move || {
                    edit_chat(&d, i, |turns, _| {
                        if let Some(t) = turns.get_mut(k) {
                            t.calls.push(ToolCall { name: String::new(), args: "{}".into() });
                        }
                    })
                })
                .class("dsx-link"),
        );
    }
    let d3 = doc.clone();
    head = head.child(icon_btn(MI_CLOSE, tr!("training.dataset.delete_turn"), move || {
        edit_chat(&d3, i, |turns, _| {
            if k < turns.len() {
                turns.remove(k);
            }
        })
    }));

    let d2 = doc.clone();
    let placeholder = match role.as_str() {
        "assistant" if !t.calls.is_empty() => "training.dataset.placeholder.assistant_call",
        "assistant" => "training.dataset.placeholder.assistant",
        "system" => "training.dataset.placeholder.system",
        "tool" => "training.dataset.placeholder.tool",
        _ => "training.dataset.placeholder.user",
    };
    let edit = text_area(t.content.clone(), tr!(placeholder), role == "tool", 20, move |s: &str| {
        let s = s.to_string();
        with_turn(&d2, i, k, |t| t.content = s);
    });

    let mut body = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head).child(edit);
    for (c, call) in t.calls.iter().enumerate() {
        body = body.child(call_view(doc.clone(), i, k, c, call));
    }
    let avatar = DecoratedBox::new()
        .class(format!("dsx-avatar dsx-avatar-{role}"))
        .child(Text::new(role_icon(&role)).class(&format!("dsx-glyph dsx-glyph-{role}")));
    mgui! {
        Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Start).class("dsx-turn") => [
            avatar,
            DecoratedBox::new().class("grow").child(body),
        ]
    }
}

/// Вызов инструмента: имя функции и аргументы JSON.
fn call_view(doc: DatasetDoc, i: usize, k: usize, c: usize, call: &ToolCall) -> impl Widget {
    let (dn, da, dr) = (doc.clone(), doc.clone(), doc);
    let args = text_area(call.args.clone(), tr!("training.dataset.placeholder.args"), true, 14, move |s: &str| {
        let s = s.to_string();
        with_turn(&da, i, k, |t| if let Some(x) = t.calls.get_mut(c) { x.args = s });
    });
    mgui! {
        DecoratedBox::new().class("dsx-call") => [
            Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Text::new("ƒ").class("dsx-call-f"),
                    DecoratedBox::new().class("dsx-name").child(
                        TextField::new().text(call.name.clone()).placeholder(tr!("training.dataset.placeholder.tool_name")).width(220.0).on_change(move |s: &str| {
                            let s = s.to_string();
                            with_turn(&dn, i, k, |t| if let Some(x) = t.calls.get_mut(c) { x.name = s });
                        })
                    ),
                    DecoratedBox::new().class("grow"),
                    icon_btn(MI_CLOSE, tr!("training.dataset.delete_call"), move || {
                        edit_chat(&dr, i, |turns, _| {
                            if let Some(t) = turns.get_mut(k) {
                                if c < t.calls.len() {
                                    t.calls.remove(c);
                                }
                            }
                        })
                    }),
                ],
                args,
            ]
        ]
    }
}
