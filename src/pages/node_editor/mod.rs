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

pub mod controls;
pub mod eval;
pub mod node_view;
pub mod nodes;
pub mod persist;
pub mod registry;
pub mod models_panel;
pub mod run_controls;
pub mod state;
pub mod style_dialog;
pub mod tabs;
pub mod template_preview;
pub mod timing;
pub mod types;
pub mod wires;

use syngui::input::MouseButton;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::containers::{GestureDetector, PanZoomViewport};
use syngui::widgets::overlay::menu::{MenuItem, PopupMenu};
use syngui::widgets::{DecoratedBox, Padding, Reactive, Row, Stack, TextField, ToolButton};

use crate::components::panel_header::{self, CenterSpec};
use crate::components::workspace_frame::{self, expand, FrameSpec};
use crate::icons::{
    MI_DASHBOARD_CUSTOMIZE, MI_FIT_SCREEN, MI_HUB, MI_PSYCHOLOGY, MI_SAVE, MI_TUNE, MI_ZOOM_IN,
    MI_ZOOM_OUT,
};

use self::registry::{NodeCategory, REGISTRY};
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

fn canvas_for(ctx: NodeEditorCtx) -> impl Widget {
    let menu_ctx = ctx;
    let filter_ctx = ctx;
    let viewport = PanZoomViewport::new()
        .pan(ctx.pan)
        .zoom(ctx.zoom)
        .zoom_range(0.25, 4.0)
        .grid(true)
        .grid_step(40.0)
        .pan_button(MouseButton::Middle)
        // LMB-pan не активируется в 10px-зоне вокруг любой ноды — иначе
        // случайный клик возле карточки начинает pan вместо drag-handle'а
        // или ловли port'а. `world` приходит в local-content-coord
        // (canvas-local), та же система, что `node.bounds`.
        .pan_filter(move |world| {
            const INFLATE: f32 = 10.0;
            let nodes = filter_ctx.nodes.get_untracked();
            !nodes.iter().any(|n| {
                let b = n.bounds.get_untracked();
                let pos = n.pos.get_untracked();
                // bounds.origin published в position-pass-coord, а world здесь
                // — в canvas-local; используем `pos` как canvas-local-origin
                // ноды (Stack кладёт Positioned в Stack.origin + node.pos).
                let left = pos.x - INFLATE;
                let top = pos.y - INFLATE;
                let right = pos.x + b.size.width + INFLATE;
                let bottom = pos.y + b.size.height + INFLATE;
                world.x >= left && world.x <= right
                    && world.y >= top && world.y <= bottom
            })
        })
        .on_background_context_menu(move |world, screen| {
            menu_ctx.open_bg_menu(world, screen);
        })
        .child(world_layer(ctx))
        .class("node-editor-viewport");

    let toolbar = build_toolbar(ctx);
    let bg_menu = build_bg_menu(ctx);
    let overlay = overlay_row(ctx, toolbar);

    let frame_inner = Stack::new().children(vec![
        Box::new(viewport) as Box<dyn Widget>,
        Box::new(overlay),
        Box::new(models_panel_host()) as Box<dyn Widget>,
        bg_menu,
        Box::new(style_dialog::view(ctx)) as Box<dyn Widget>,
    ]);

    DecoratedBox::new().child(frame_inner).class("ne-canvas-frame")
}

/// Верхняя «overlay-полоса» canvas'а: zoom-toolbar слева. Run-пилюля
/// переехала в общую шапку страницы.
///
/// **Pass-through events**: возвращаем `Padding` напрямую (без DecoratedBox-
/// обёртки). Stack растянет padding на всю canvas, но прозрачные layout-
/// контейнеры (Padding/Row) не ловят события — клик/drag/zoom в пустом месте
/// уходит на viewport ниже. DecoratedBox в overlay-host раньше перехватывал
/// все события и блокировал pan/zoom — поэтому его и нет.
///
fn overlay_row(_ctx: NodeEditorCtx, toolbar: Box<dyn Widget>) -> impl Widget {
    let row = Row::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .main_axis_alignment(MainAxisAlignment::Start)
        .children(vec![toolbar]);

    // Padding без DecoratedBox-обёртки: Padding/Row реализуют
    // `passthrough_hit_test() == true`, поэтому пустые места строки
    // пропускают события дальше в Stack (viewport получает pan/zoom/click).
    // Row.cross_axis_alignment(Start) делает строку intrinsic-высоты —
    // нижняя часть canvas остаётся свободной для drag-нод.
    Padding::only(16.0, 16.0, 16.0, 0.0).child(row)
}

/// Хост панели «Модели в памяти»: правый край canvas'а, ниже overlay-строки
/// с toolbar'ом и run-pill'ом. Как и `overlay_row`, возвращаем Padding без
/// DecoratedBox-обёртки — Padding/Row passthrough'ат hit-test, поэтому
/// пустое место справа от панели по-прежнему пан'ит и зумит canvas.
fn models_panel_host() -> impl Widget {
    Padding::only(0.0, 68.0, 16.0, 16.0).child(
        Row::new()
            .gap(0.0)
            .main_axis_alignment(MainAxisAlignment::End)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .children(vec![Box::new(models_panel::view()) as Box<dyn Widget>]),
    )
}

/// Реактивный «мир» — Canvas wires + ноды на абсолютных позициях.
fn world_layer(ctx: NodeEditorCtx) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        // Подмена контекста: внутри Reactive use_context::<NodeEditorCtx>()
        // должно вернуть именно эту вкладку. provide_context на уровне
        // Reactive children — единственное место, где это можно сделать
        // согласованно для wires::view() и node_view::view(node).
        provide_context(ctx);
        let nodes = ctx.nodes.get();
        let mut stack = Stack::new();
        stack = stack.child(wires::view());
        for node in nodes {
            stack = stack.children(vec![node_view::view(node)]);
        }
        // Без DecoratedBox-обёртки: оборачивание в DecoratedBox с прозрачным
        // фоном лишь ставило не-passthrough контейнер между Reactive и Stack —
        // в моменты `bounds=(0,0)` (rebuild) hit-test обрезался на нём, ноды
        // не получали клики после pan/scroll. Stack passthrough даёт прямой
        // путь к Positioned'ам и далее к карточкам нод.
        vec![Box::new(stack) as Box<dyn Widget>]
    })
}

fn build_toolbar(ctx: NodeEditorCtx) -> Box<dyn Widget> {
    let ctx_in = ctx;
    let ctx_out = ctx;
    let ctx_fit = ctx;

    // DecoratedBox-host даёт Reactive фиксированные constraints (через MSS
     // width/height в `.node-editor-toolbar-zoom-host`). Без обёртки
    // Reactive.layout получает max_width=INF от Row и схлопывается до 0×0
    // — Text внутри не рисуется. См. Reactive.layout в syngui.
    let label = DecoratedBox::new()
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let z = ctx_in.zoom.get();
            let pct = (z * 100.0).round() as i32;
            vec![Box::new(Text::new(format!("{}%", pct)).class("node-editor-toolbar-zoom"))]
        }))
        .class("node-editor-toolbar-zoom-host");

    let row = mgui! {
        Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                ToolButton::new(MI_ZOOM_OUT)
                    .tooltip(tr!("nodes.toolbar.zoom_out"))
                    .on_click(move || {
                        let z = (ctx_out.zoom.get_untracked() * 0.8).max(0.25);
                        ctx_out.zoom.set(z);
                    })
                    .class("node-editor-toolbar-btn"),
                label,
                ToolButton::new(MI_ZOOM_IN)
                    .tooltip(tr!("nodes.toolbar.zoom_in"))
                    .on_click(move || {
                        let z = (ctx_in.zoom.get_untracked() * 1.25).min(4.0);
                        ctx_in.zoom.set(z);
                    })
                    .class("node-editor-toolbar-btn"),
                ToolButton::new(MI_FIT_SCREEN)
                    .tooltip(tr!("nodes.toolbar.reset_view"))
                    .on_click(move || ctx_fit.reset_view())
                    .class("node-editor-toolbar-btn"),
            ]
    };

    let panel = DecoratedBox::new()
        .child(Padding::symmetric(8.0, 4.0).child(row))
        .class("node-editor-toolbar");

    Box::new(panel)
}

/// PopupMenu «Add Node» — открывается через ctx.menu_open + позиция в
/// ctx.menu_world_pos. Top-level — категории, внутри каждой категории —
/// либо leaf-пункты (для нод без `subcategory`), либо submenu по
/// `subcategory` (для группировки EQ и пр.). Mixer обрабатывается
/// отдельно — его 5 пресетов (2/3/4/5 входа + Custom) живут в подменю
/// «Аудио микшеры» внутри DSP-эффектов и стартуют ноду с нужным
/// `n_inputs`.
fn build_bg_menu(ctx: NodeEditorCtx) -> Box<dyn Widget> {
    use crate::icons::MI_MERGE_TYPE;
    use crate::pages::node_editor::types::NodeKind;
    use std::collections::BTreeMap;

    let items: Vec<MenuItem> = NodeCategory::ORDER
        .iter()
        .filter_map(|cat| {
            // Top-level: ноды этой категории без subcategory и (для DspEffects)
            // не Mixer (он переезжает в спец-пункт).
            let mut top: Vec<MenuItem> = REGISTRY
                .iter()
                .filter(|m| {
                    m.category == *cat
                        && m.subcategory.is_none()
                        && !(matches!(m.kind, NodeKind::Mixer) && *cat == NodeCategory::DspEffects)
                })
                .map(|m| MenuItem::new(format!("add:{}", m.title), crate::i18n::node_title(m)).icon(m.icon))
                .collect();

            // Submenu по subcategory.
            let mut by_sub: BTreeMap<&'static str, Vec<&'static registry::NodeKindMeta>> =
                BTreeMap::new();
            for m in REGISTRY.iter().filter(|m| m.category == *cat) {
                if let Some(sub) = m.subcategory {
                    by_sub.entry(sub).or_default().push(m);
                }
            }
            for (sub_key, metas) in by_sub {
                let icon = metas.first().map(|m| m.icon).unwrap_or("");
                let children: Vec<MenuItem> = metas
                    .iter()
                    .map(|m| MenuItem::new(format!("add:{}", m.title), crate::i18n::node_title(*m)).icon(m.icon))
                    .collect();
                top.push(
                    MenuItem::new(format!("sub:{}:{}", cat.key(), sub_key), crate::i18n::node_subcategory_label(sub_key))
                        .icon(icon)
                        .children(children),
                );
            }

            // Спец-обработка Mixer: 5 пресетов (2/3/4/5 + Custom).
            if *cat == NodeCategory::DspEffects {
                let mixer_presets = vec![
                    MenuItem::new("add_mixer:2", trn!("nodes.menu.mixer_sources", 2)),
                    MenuItem::new("add_mixer:3", trn!("nodes.menu.mixer_sources", 3)),
                    MenuItem::new("add_mixer:4", trn!("nodes.menu.mixer_sources", 4)),
                    MenuItem::new("add_mixer:5", trn!("nodes.menu.mixer_sources", 5)),
                    MenuItem::new("add_mixer:custom", tr!("nodes.menu.mixer_custom")),
                ];
                top.push(
                    MenuItem::new("sub:mixer", tr!("nodes.menu.mixer_group"))
                        .icon(MI_MERGE_TYPE)
                        .children(mixer_presets),
                );
            }

            if top.is_empty() {
                return None;
            }
            Some(
                MenuItem::new(format!("cat:{:?}", cat), crate::i18n::node_category_label(*cat))
                    .icon(cat.icon())
                    .children(top),
            )
        })
        .collect();

    let ctx_select = ctx.clone();
    Box::new(
        PopupMenu::new()
            .is_open(ctx.menu_open)
            .position(ctx.menu_screen_pos)
            .items(items)
            .on_select(move |action| {
                if let Some(rest) = action.strip_prefix("add:") {
                    let kind_opt = REGISTRY.iter().find(|m| m.title == rest).map(|m| m.kind);
                    if let Some(k) = kind_opt {
                        let world = ctx_select.menu_world_pos.get_untracked();
                        ctx_select.add_node(k, world);
                    }
                } else if let Some(rest) = action.strip_prefix("add_mixer:") {
                    let n: usize = match rest {
                        "custom" => 2,
                        s => s.parse().unwrap_or(2),
                    };
                    let world = ctx_select.menu_world_pos.get_untracked();
                    let id = ctx_select.add_node(NodeKind::Mixer, world);
                    ctx_select.set_mixer_n_inputs(id, n);
                }
                ctx_select.menu_open.set(false);
            }),
    )
}
