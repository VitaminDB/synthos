//! Страница редактора нод.
//!
//! Layout:
//! - workspace_frame без боковых панелей (канвас на всю ширину):
//!   1. заголовок (panel_header::center) — название графа (inline-rename),
//!      шаблон-источник, поиск, Run/Pause/Stop с таймером, «Шаблоны»,
//!      «Сохранить как шаблон».
//!   2. canvas-area:
//!      Stack:
//!        - PanZoomViewport (с padding-frame, dot-grid не цепляется к
//!          границам shell'а).
//!        - Column[overlay_row, Spacer]: overlay_row = Row[Toolbar] —
//!          zoom-toolbar слева сверху канваса.
//!        - Панель «Модели в памяти» справа сверху.
//!        - PopupMenu (Add Node) — overlay через `is_open` сигнал ctx.
//!
//! Открытые графы — плитки нав-рейла (`crate::rail`), полосы вкладок на
//! странице нет. Каждый граф владеет своим [`NodeEditorCtx`]. View
//! переключается через `EditorWorkspace.active` — реактивно через
//! Reactive-обёртки.

pub mod models_panel;

// Сам редактор узлов — общий крейт synshell `syn-node-editor` (им же
// пользуется видеоредактор synshell). Здесь — страница Synthos вокруг
// холста: шапка, шаблоны, панель моделей, и хост редактора.
pub use syn_node_editor::{
    canvas, controls, eval, node_view, nodes, persist, registry, run_controls, state, style_dialog, tabs,
    template_preview, timing, types, wires,
};

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::{DecoratedBox, Padding, Reactive, Row, Stack, TextField};

use crate::components::panel_header::{self, CenterSpec};
use crate::components::workspace_frame::{self, expand, FrameSpec};
use crate::icons::{MI_DASHBOARD_CUSTOMIZE, MI_HUB, MI_PSYCHOLOGY, MI_SAVE, MI_TUNE};

use self::state::NodeEditorCtx;
use self::tabs::{EditorWorkspace, OpenTab};

/// Корневой view страницы — каркас без боковых панелей: заголовок + канвас.
pub fn view() -> impl Widget {
    DecoratedBox::new()
        .child(workspace_frame::view(FrameSpec::new(
            "node-editor-h-split",
            || Box::new(header()),
            || Box::new(canvas_area()),
        )))
        .class("node-editor-root")
}

// ───────────────────────────── Заголовок ─────────────────────────────

fn header() -> impl Widget {
    let identity: Box<dyn Widget> = Box::new(DecoratedBox::new().child(identity_reactive));
    let actions = DecoratedBox::new().child(actions_reactive);
    panel_header::center(CenterSpec::new(identity).actions(actions))
}

/// Активный граф: иконка по происхождению, название с inline-rename,
/// подзаголовок — шаблон-источник и признак несохранённых правок.
fn identity_reactive() -> Stack {
    let ws = use_context::<EditorWorkspace>();
    let active = ws.active.get();
    let tab = active.and_then(|id| ws.tabs.get().into_iter().find(|t| t.id == id));
    let Some(tab) = tab else {
        return expand(panel_header::identity_text(
            MI_HUB,
            tr!("nav.nodes"),
            tr!("nodes.empty.title"),
        ));
    };
    let icon = if tab.agent_chat.get_untracked().is_some() {
        MI_PSYCHOLOGY
    } else if tab.source.get_untracked().is_some() {
        MI_HUB
    } else {
        MI_TUNE
    };
    expand(panel_header::identity(
        panel_header::icon_bubble(icon),
        DecoratedBox::new().child(move || title_block(tab)),
        DecoratedBox::new().child(move || subtitle_block(tab)),
    ))
}

/// Название графа: клик — поле ввода, Enter — новое имя.
fn title_block(tab: OpenTab) -> StyledWidget<DecoratedBox> {
    let editing = tab_editing_flag(tab.id.0);
    let title = tab.title.get();
    if editing.get() {
        let field = TextField::new()
            .text(title)
            .on_submit(move |s| {
                let t = s.trim().to_string();
                if !t.is_empty() {
                    tab.title.set(t);
                }
                editing.set(false);
            })
            .class("chat-header-title-edit");
        DecoratedBox::new().class("chat-header-title-wrap").child(field)
    } else {
        let clickable = GestureDetector::new()
            .on_click(move || editing.set(true))
            .child(Text::new(title).max_lines(1).class("panel-header-title"));
        DecoratedBox::new().class("chat-header-title-wrap").child(clickable)
    }
}

/// Флаг «идёт переименование» на граф — в статике, потому что шапка
/// пересобирается роутером и локальный сигнал терял бы состояние.
fn tab_editing_flag(id: u64) -> RwSignal<bool> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static FLAGS: OnceLock<Mutex<HashMap<u64, RwSignal<bool>>>> = OnceLock::new();
    let map = FLAGS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    *guard.entry(id).or_insert_with(|| use_signal(false))
}

fn subtitle_block(tab: OpenTab) -> DecoratedBox {
    let dirty = tab.dirty.get();
    let source = tab.source.get();
    let mut text = match source.as_deref().and_then(template_title) {
        Some(name) => tr!("nodes.header.from_template", name = name),
        None if tab.agent_chat.get_untracked().is_some() => tr!("nodes.header.agent_graph"),
        None => tr!("nodes.header.scratch"),
    };
    // Страница всегда автосохраняется в workspace.json — «не сохранено»
    // здесь врало бы. Метка означает «отличается от своего шаблона».
    if dirty {
        text.push_str(" · ");
        text.push_str(&tr!("nodes.header.modified"));
    }
    DecoratedBox::new().child(Text::new(text).max_lines(1).class("panel-header-subtitle"))
}

fn template_title(id: &str) -> Option<String> {
    crate::templates::list_all()
        .into_iter()
        .find(|t| t.id == id)
        .map(|t| crate::i18n::template_name(&t))
}

/// Действия: Run/Pause/Stop с таймером (пилюля переехала с канваса),
/// окно шаблонов, сохранить граф как шаблон.
fn actions_reactive() -> Stack {
    let ws = use_context::<EditorWorkspace>();
    let Some(ctx) = ws.active_ctx() else {
        return expand(Box::new(
            panel_header::action_button(
                MI_DASHBOARD_CUSTOMIZE,
                tr!("nodes.header.templates"),
                move || ws.template_picker_open.set(true),
            ),
        ));
    };
    let row = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            run_controls::view(ctx),
            panel_header::action_divider(),
            panel_header::action_button(
                MI_DASHBOARD_CUSTOMIZE,
                tr!("nodes.header.templates"),
                move || ws.template_picker_open.set(true),
            ),
            panel_header::action_button(MI_SAVE, tr!("nodes.header.save_template"), || {
                // Всегда новый шаблон: тот, из которого открыта страница,
                // остаётся неизменным (шаблон = константа).
                crate::components::template_picker::save_current_as_template();
            }),
        ]
    };
    expand(Box::new(row))
}

/// Активная вкладка → её Stack canvas. При смене active id Reactive
/// перестраивает дерево с новым `NodeEditorCtx`.
fn canvas_area() -> impl Widget {
    DecoratedBox::new()
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let ws = use_context::<EditorWorkspace>();
            let Some(ctx) = ws.active_ctx() else {
                return vec![Box::new(empty_state())];
            };
            vec![Box::new(canvas_for(ctx))]
        }))
        .class("ne-canvas-area")
}

fn empty_state() -> impl Widget {
    DecoratedBox::new()
        .child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .gap(8.0)
                .child(Text::new(tr!("nodes.empty.title")).class("ne-empty-title"))
                .child(Text::new(tr!("nodes.empty.hint")).class("ne-empty-hint")),
        )
        .class("ne-empty")
}

/// Холст графа (общий редактор) с панелью «Модели в памяти» Synthos.
fn canvas_for(ctx: NodeEditorCtx) -> impl Widget {
    canvas::view(ctx, vec![Box::new(models_panel_host())])
}

/// Хост панели «Модели в памяти»: правый край canvas'а, ниже overlay-строки
/// с toolbar'ом и run-pill'ом. Возвращаем Padding без DecoratedBox-обёртки —
/// Padding/Row passthrough'ат hit-test, поэтому пустое место справа от
/// панели по-прежнему пан'ит и зумит canvas.
fn models_panel_host() -> impl Widget {
    Padding::only(0.0, 68.0, 16.0, 16.0).child(
        Row::new()
            .gap(0.0)
            .main_axis_alignment(MainAxisAlignment::End)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .children(vec![Box::new(models_panel::view()) as Box<dyn Widget>]),
    )
}

// ───────────────────────────── Хост редактора ─────────────────────────────

/// Связь общего редактора с Synthos: уведомления — лентой приложения,
/// каталоги — `~/.config/synthos`, `~/.cache/synthos`, битые файлы — в
/// общий список отложенных (уведомление при старте), подписи встроенных
/// шаблонов — из каталогов строк.
struct SynthosHost;

impl syn_node_editor::host::EditorHost for SynthosHost {
    fn app_name(&self) -> &'static str {
        "synthos"
    }

    fn notify(&self, kind: syn_node_editor::host::Notice, text: String) {
        let Some(app) = try_use_context::<crate::context::AppCtx>() else {
            tracing::info!("{text}");
            return;
        };
        match kind {
            syn_node_editor::host::Notice::Info => app.notifications.info(text),
            syn_node_editor::host::Notice::Error => app.notifications.error(text),
        }
    }

    fn open_path(&self, path: &std::path::Path) -> std::io::Result<()> {
        crate::paths::open_with_system(path)
    }

    fn quarantine(&self, path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        crate::fsutil::quarantine(path)
    }

    fn template_title(&self, t: &crate::templates::Template) -> String {
        crate::i18n::template_name(t)
    }
}

/// Поставить хост редактора — при запуске, до первого обращения к графам.
pub fn install_host() {
    syn_node_editor::host::set_host(SynthosHost);
}
