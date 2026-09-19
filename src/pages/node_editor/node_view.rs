//! Карточка одной ноды: header с drag-handle, иконкой, заголовком, ×;
//! строки портов; тело с input-виджетами по схеме из registry.

use std::any::Any;
use std::sync::Arc;

use syngui::core::sync::Mutex;
use syngui::core::{Color, Point, Rect, Size};
use syngui::input::{CursorIcon, Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::mgui;
use syngui::mss::{ComputedStyle, MssFields};
use syngui::prelude::*;
use syngui::render::{Border, DisplayList};
use syngui::widget::{
    DirtyFlags, Element, ElementId, ElementTree, LayoutHint, UpdateContext, Widget,
};
use syngui::widget::context::EventContext;
use syngui::widgets::input::dropdown::DropdownItem;
use syngui::widgets::overlay::context_menu::ContextMenu;
use syngui::widgets::overlay::menu::MenuItem;
use syngui::widgets::{
    Checkbox, ColorPicker, ColorValue, Column, DecoratedBox, Dropdown, Padding, Reactive, Row,
    Slider, SpinBox, TextField, Toggle, ToolButton,
};
use syngui::widgets::containers::Positioned;

use syngui::widget::WidgetExt;

use crate::icons::{
    MI_ASPECT_RATIO, MI_CIRCLE, MI_CLOSE, MI_CONTENT_COPY, MI_EDIT_NOTE, MI_PALETTE,
    MI_POWER_SETTINGS, MI_REMOVE_CIRCLE_OUTLINE, MI_WB_SHADE,
};

use super::registry::{self, NodeKindMeta};
use super::state::NodeEditorCtx;
use super::types::{
    FieldSchema, FieldType, FieldValue, NodeId, NodeInstance, NodeKind, NodeStyle, PortKind,
    PortSchema, PortSide, PortsLayout,
};
use super::wires::wire_color;

// ── Геометрия карточки ──────────────────────────────────────────────────
//
// Размер карточки полностью определяется layout-системой syngui через
// `width: fit-content; height: fit-content;` в `.node-card` (MSS).
// Реальные world-координаты PortDot'ов публикуются ими самими в
// `NodeEditorCtx::port_positions` после каждого `set_position`.
// Никаких magic-чисел в `NodeKindMeta::fixed_*`.
const HEADER_HEIGHT: f32 = 32.0;
const PORT_ROW_HEIGHT: f32 = 22.0;
const PORT_DOT: f32 = 12.0;
const NODE_PADDING_H: f32 = 10.0;
const BODY_PADDING_V: f32 = 6.0;


/// World-position указанного порта ноды. Возвращает `(point, kind)`. Используется
/// в `wires::draw_*` для построения bezier-кривой между портами.
pub fn port_world_pos(node: &NodeInstance, side: PortSide, port_name: &str) -> Option<(Point, PortKind)> {
    let meta = registry::meta(node.kind);
    let list: &[PortSchema] = match side {
        PortSide::Input => meta.inputs.resolve(node),
        PortSide::Output => meta.outputs.resolve(node),
    };
    let idx = list.iter().position(|p| p.name == port_name)?;
    let p = list[idx];
    // Реактивная подписка на pos и size — wires автоматически перерисуются
    // при drag'е ноды или shrink-to-fit карточки. node.size публикуется
    // SizeReport-обёрткой (ResizeObserver-аналог) в node_view::view.
    // Координаты в local-pre-transform-системе PanZoomViewport — совпадают
    // с системой, в которой рисуется wires-Canvas.
    // Координаты в **canvas-local** системе, в которой `CanvasContext::draw_*`
    // интерпретирует переданные числа (origin = canvas.bounds.origin).
    // Canvas wires — child of Stack, и Stack кладёт Positioned-children в
    // `Stack.origin + node.pos` — т.е. node.pos и есть canvas-local-coord
    // ноды. Размер карточки берём из node.bounds (опубликованного SizeReport),
    // высота для inline_ports — от центра body.
    //
    // `update_wire(*pos)` получает adjusted_pos в position-pass-coord
    // (= canvas.bounds.origin + canvas-local). Преобразование в canvas-local
    // выполняется в `update_wire` через вычитание canvas_origin —
    // см. `state::canvas_origin` helper.
    let pos = node.pos.get();
    let bounds = node.bounds.get();
    let w = bounds.size.width;
    let h = bounds.size.height;
    let x = match side {
        PortSide::Input => pos.x,
        PortSide::Output => pos.x + w,
    };
    let y = match meta.ports_layout {
        PortsLayout::None => pos.y + (HEADER_HEIGHT + h) * 0.5,
        PortsLayout::Rows => {
            pos.y + HEADER_HEIGHT + BODY_PADDING_V + PORT_ROW_HEIGHT * (idx as f32 + 0.5)
        }
        PortsLayout::Compact { centered: true } => pos.y + (HEADER_HEIGHT + h) * 0.5,
        PortsLayout::Compact { centered: false } => {
            if side == PortSide::Input {
                let body_h = (h - HEADER_HEIGHT - 2.0 * BODY_PADDING_V).max(0.0);
                let count = list.len().max(1) as f32;
                let row_h = body_h / count;
                pos.y + HEADER_HEIGHT + BODY_PADDING_V + row_h * (idx as f32 + 0.5)
            } else {
                pos.y + (HEADER_HEIGHT + h) * 0.5
            }
        }
    };
    Some((Point::new(x, y), p.kind))
}

/// Вернуть widget — карточку для одной ноды как `Positioned`, привязанный
/// к `node.pos` через `offset_signal`. Размер shrink-to-fit'ится по
/// контенту через MSS `width: fit-content; height: fit-content;` на
/// `.node-card` — `Positioned` БЕЗ `.size()` пробрасывает child loose-
/// constraints, `measure_container` возвращает реальный размер.
pub fn view(node: NodeInstance) -> Box<dyn Widget> {
    let meta = registry::meta(node.kind);
    // SizeReport-обёртка публикует фактический layout-size карточки в
    // `node.size` — `port_world_pos` использует его для координат портов.
    // Это аналог CSS `ResizeObserver` API: layout публикует размер наружу
    // через signal, чтобы код вне дерева (wires-Canvas) мог реагировать.
    let card = SizeReport::new(node.bounds, node_card(node.clone(), meta));
    Box::new(Positioned::new(card).offset_signal(node.pos))
}

/// Базовый фон карточки `.node-card` (MSS `#2A2D38`). Используется как
/// основа для blend'а с `NodeStyle.tint` — окрашенный фон не теряет
/// «node-look» (тёмный), но получает узнаваемый акцент.
fn base_card_color() -> Color {
    Color::from_hex("#2A2D38")
}

/// Линейный blend между двумя цветами в sRGB-пространстве. `alpha` —
/// доля `tint` в результате (0 = чистый base, 1 = чистый tint). Для
/// node-tint используем `alpha ≈ 0.25` — заметно, но не кричаще.
fn blend_tint(base: Color, tint: Color, alpha: f32) -> Color {
    let a = alpha.clamp(0.0, 1.0);
    Color::new(
        base.r * (1.0 - a) + tint.r * a,
        base.g * (1.0 - a) + tint.g * a,
        base.b * (1.0 - a) + tint.b * a,
        1.0,
    )
}

fn node_card(node: NodeInstance, meta: &'static NodeKindMeta) -> impl Widget {
    let id = node.id;
    let kind = node.kind;
    let style_signal = node.style;
    let enabled_signal = node.enabled;
    let node_for_menu = node.clone();
    let node_for_body = node.clone();

    // Карточка пересобирается реактивно при изменении style/enabled —
    // это переустанавливает inline-style background-color (tint blend)
    // и динамические классы (disabled / no-shadow). ContextMenu тоже
    // внутри — items с текущими toggle-метками («Тень: ✓/☐»,
    // «Отключить/Включить») генерируются на момент следующего mount меню.
    //
    // Body (header + ports + fields) пересоздаётся вместе с карточкой,
    // но всё реактивное состояние ноды живёт в `node.fields/runtime/style/
    // enabled` (Copy-RwSignal, shared), поэтому пересборка UI не теряет
    // ни одного пользовательского значения.
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let style: NodeStyle = style_signal.get();
        let enabled: bool = enabled_signal.get();

        let body = Column::new()
            .gap(0.0)
            .children(vec![
                Box::new(header_widget(
                    id,
                    meta,
                    node_for_body.pos,
                    node_for_body.timing,
                )) as Box<dyn Widget>,
                ports_and_fields(node_for_body.clone(), meta),
            ]);

        // Базовая карточка — DecoratedBox с MSS-классом. Динамические классы
        // навешиваем через WidgetExt::class (StyledWidget-обёртка).
        let mut classes: Vec<String> = vec!["node-card".to_string()];
        if matches!(kind, NodeKind::MarkdownView) {
            classes.push("markdown-node".to_string());
        }
        if matches!(kind, NodeKind::OmniVoice) {
            classes.push("omnivoice-node".to_string());
        }
        match kind {
            NodeKind::VoxCpm2 => classes.push("voxcpm-node".to_string()),
            NodeKind::VibeVoice => classes.push("vibevoice-node".to_string()),
            NodeKind::Llm => classes.push("llm-node".to_string()),
            NodeKind::AceStepVaeEncode => classes.push("acestep-vae-encode-node".to_string()),
            NodeKind::AceStepCheckpoint => classes.push("acestep-checkpoint-node".to_string()),
            NodeKind::AceStepGenerate => classes.push("acestep-generate-node".to_string()),
            NodeKind::H3Checkpoint => classes.push("h3-checkpoint-node".to_string()),
            NodeKind::H3TextEncoder => classes.push("h3-text-encoder-node".to_string()),
            NodeKind::H3EmptyLatentAv => classes.push("h3-empty-latent-node".to_string()),
            NodeKind::H3Keyframe => classes.push("h3-keyframe-node".to_string()),
            NodeKind::H3References => classes.push("h3-references-node".to_string()),
            NodeKind::H3Sampler => classes.push("h3-sampler-node".to_string()),
            NodeKind::H3VaeDecode => classes.push("h3-vae-decode-node".to_string()),
            NodeKind::H3AudioDecode => classes.push("h3-audio-decode-node".to_string()),
            NodeKind::H3VideoSave => classes.push("h3-video-save-node".to_string()),
            NodeKind::FluxCheckpoint
            | NodeKind::FluxTextEncoder
            | NodeKind::FluxEmptyLatent
            | NodeKind::FluxVaeEncode
            | NodeKind::FluxSampler
            | NodeKind::FluxVaeDecode => classes.push("flux-node".to_string()),
            NodeKind::ImageLoad | NodeKind::ImageSave => classes.push("image-node".to_string()),
            NodeKind::LtxCheckpoint => classes.push("ltx-checkpoint-node".to_string()),
            NodeKind::LtxTextEncoder => classes.push("ltx-text-encoder-node".to_string()),
            NodeKind::LtxNagPrompt => classes.push("ltx-nag-node".to_string()),
            NodeKind::LtxSamplerStage1 => classes.push("ltx-sampler1-node".to_string()),
            NodeKind::LtxUpscale => classes.push("ltx-upscale-node".to_string()),
            NodeKind::LtxSamplerStage2 => classes.push("ltx-sampler2-node".to_string()),
            NodeKind::LtxVaeDecode => classes.push("ltx-vae-decode-node".to_string()),
            NodeKind::LtxAudioDecode => classes.push("ltx-audio-decode-node".to_string()),
            NodeKind::LtxVideoSave => classes.push("ltx-video-save-node".to_string()),
            NodeKind::LtxImage => classes.push("ltx-image-node".to_string()),
            NodeKind::LtxVideoInput => classes.push("ltx-video-input-node".to_string()),
            NodeKind::LtxRetake => classes.push("ltx-retake-node".to_string()),
            NodeKind::LtxIcLora => classes.push("ltx-iclora-node".to_string()),
            NodeKind::LtxAudioInput => classes.push("ltx-audio-input-node".to_string()),
            NodeKind::LtxLipdub => classes.push("ltx-lipdub-node".to_string()),
            NodeKind::LtxA2V => classes.push("ltx-a2v-node".to_string()),
            _ => {}
        }
        if !enabled {
            classes.push("disabled".to_string());
        }
        // Shadow выключается явно пользователем ИЛИ автоматически у
        // disabled-ноды — disabled-карточка без тени читается как
        // «выпавшая из сцены», без дублирующего drop-shadow.
        if !style.shadow || !enabled {
            classes.push("no-shadow".to_string());
        }

        let card_box = DecoratedBox::new().child(body);
        let mut styled = card_box.classes(classes);
        if let Some(tint) = style.tint {
            // CSS-переменная `--node-bg` читается из `.node-card`-правил
            // в base- и hover-вариантах — это устраняет мерцание на hover
            // (без переменной inline-style background-color терялся при
            // recomputed hover-state и фон откатывался на дефолтный).
            let bg = blend_tint(base_card_color(), tint, 0.25);
            styled = styled.style("--node-bg", bg);
        }

        // Меню — items пересоздаются при каждом перестроении карточки,
        // что синхронизирует текст toggle-пунктов с текущим состоянием.
        let mut items = vec![
            MenuItem::new("delete", tr!("app.delete")).icon(MI_CLOSE),
            MenuItem::new("duplicate", tr!("nodes.menu.duplicate")).icon(MI_CONTENT_COPY),
            MenuItem::new("disconnect", tr!("nodes.menu.disconnect")).icon(MI_CIRCLE),
        ];
        if matches!(kind, NodeKind::MarkdownView) {
            items.push(MenuItem::new("md-edit", tr!("nodes.menu.edit")).icon(MI_EDIT_NOTE));
            items.push(MenuItem::new("md-resize", tr!("nodes.menu.resize")).icon(MI_ASPECT_RATIO));
        }
        if matches!(kind, NodeKind::TextView) {
            items.push(MenuItem::new("tv-resize", tr!("nodes.menu.resize")).icon(MI_ASPECT_RATIO));
        }
        items.push(MenuItem::new("tint-open", tr!("nodes.menu.tint_open")).icon(MI_PALETTE));
        if style.tint.is_some() {
            items.push(
                MenuItem::new("tint-reset", tr!("nodes.menu.tint_reset")).icon(MI_REMOVE_CIRCLE_OUTLINE),
            );
        }
        let shadow_mark = if style.shadow { "✓" } else { "☐" };
        let shadow_label = tr!("nodes.menu.shadow", mark = shadow_mark);
        items.push(MenuItem::new("shadow-toggle", shadow_label).icon(MI_WB_SHADE));
        let enabled_label = if enabled { tr!("nodes.menu.disable") } else { tr!("nodes.menu.enable") };
        items.push(MenuItem::new("enabled-toggle", enabled_label).icon(MI_POWER_SETTINGS));

        let node_for_select = node_for_menu.clone();
        let menu = ContextMenu::new()
            .items(items)
            .on_select(move |action| {
                let nctx = use_context::<NodeEditorCtx>();
                match action {
                    "delete" => nctx.remove_node(id),
                    "duplicate" => nctx.duplicate_node(id),
                    "disconnect" => nctx.disconnect_all(id),
                    "md-edit" => super::nodes::markdown_view::toggle_edit_mode(&node_for_select),
                    "md-resize" => super::nodes::markdown_view::toggle_resize_mode(&node_for_select),
                    "tv-resize" => super::nodes::text_view::toggle_resize_mode(&node_for_select),
                    "tint-open" => nctx.tint_dialog.set(Some(id)),
                    "tint-reset" => {
                        node_for_select.style.update(|s| s.tint = None);
                    }
                    "shadow-toggle" => {
                        node_for_select.style.update(|s| s.shadow = !s.shadow);
                    }
                    "enabled-toggle" => {
                        let cur = node_for_select.enabled.get_untracked();
                        node_for_select.enabled.set(!cur);
                    }
                    _ => {}
                }
            })
            .child(styled);

        vec![Box::new(menu)]
    })
}

fn header_widget(
    id: NodeId,
    meta: &'static NodeKindMeta,
    pos_signal: RwSignal<Point>,
    timing: super::timing::Stopwatch,
) -> impl Widget {
    let title = crate::i18n::node_title(meta);
    let icon = meta.icon;
    // Title-group слева (icon + title) и close-кнопка справа в общем Row
    // с MainAxisAlignment::SpaceBetween — распределение к краям не зависит
    // от того, делает ли Row shrink-to-fit. Раньше использовался
    // `FieldFlexSpacer` (flex-grow:1), но при некоторых раскладках
    // (multi-tab node-editor) Row внутри DragHandle не получал tight
    // constraints от родителя → shrink-to-fit активировался → spacer не
    // растягивался → close уезжал к центру header'а.
    let title_group = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![
            Box::new(Text::new(icon).class("node-card-icon")) as Box<dyn Widget>,
            Box::new(Text::new(title).class("node-card-title")) as Box<dyn Widget>,
        ]);

    let close_btn = ToolButton::new(MI_CLOSE)
        .on_click(move || {
            use_context::<NodeEditorCtx>().remove_node(id);
        })
        .class("node-card-close");

    // Правая группа шапки: [таймер] ×. Бейдж таймера ставится только у
    // нод с явным запуском (у них есть `busy_signal` — по нему и меряется
    // длительность); у реактивных Number/Add мерить нечего. Сам бейдж
    // прячется, пока у ноды не было ни одного прогона.
    let mut tail: Vec<Box<dyn Widget>> = Vec::new();
    if meta.busy_signal.is_some() {
        tail.push(super::timing::node_timer_badge(timing));
    }
    tail.push(Box::new(close_btn) as Box<dyn Widget>);
    let tail_group = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(tail);

    let inner_row = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .children(vec![
            Box::new(title_group) as Box<dyn Widget>,
            Box::new(tail_group) as Box<dyn Widget>,
        ]);
    let inner_padded = Padding::symmetric(NODE_PADDING_H, 0.0).child(inner_row);
    DragHandle::new(pos_signal, HEADER_HEIGHT, inner_padded)
}

fn ports_and_fields(node: NodeInstance, meta: &'static NodeKindMeta) -> Box<dyn Widget> {
    match meta.ports_layout {
        PortsLayout::None => {
            if let Some(builder) = meta.body {
                let body_widget = builder(&node);
                return Box::new(
                    DecoratedBox::new()
                        .child(Column::new().gap(0.0).children(vec![body_widget]))
                        .class("node-card-body"),
                );
            }
        }
        PortsLayout::Compact { centered } => {
            return Box::new(compact_layout(node, meta, centered));
        }
        PortsLayout::Rows => {}
    }

    let mut col = Column::new().gap(0.0);

    let inputs = meta.inputs.resolve(&node);
    let outputs = meta.outputs.resolve(&node);
    let port_count = inputs.len().max(outputs.len());
    for i in 0..port_count {
        let in_port = inputs.get(i).copied();
        let out_port = outputs.get(i).copied();
        col = col.children(vec![port_row(node.id, in_port, out_port)]);
    }

    if let Some(builder) = meta.body {
        let body_widget = builder(&node);
        col = col.child(DecoratedBox::new().class("node-card-divider"));
        col = col.children(vec![body_widget]);
        return Box::new(DecoratedBox::new().child(col).class("node-card-body"));
    }

    // Вычисленное значение — для Output крупно, для Add/Number мелко.
    match meta.kind {
        NodeKind::Output => {
            col = col.children(vec![value_display(node.id, "in", "node-output-value")]);
        }
        NodeKind::Add | NodeKind::Number => {
            col = col.children(vec![value_display(node.id, "out", "node-port-value")]);
        }
        _ => {}
    }

    if !meta.fields.is_empty() {
        col = col.child(DecoratedBox::new().class("node-card-divider"));
        let fields = node.fields.clone();
        for f in meta.fields {
            col = col.children(vec![field_row(*f, fields.clone())]);
        }
    }
    Box::new(DecoratedBox::new().child(col).class("node-card-body"))
}

fn compact_port_in(node_id: NodeId, schema: PortSchema) -> Box<dyn Widget> {
    let color = wire_color(schema.kind, false);
    Box::new(mgui! {
        Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                PortDot::new(node_id, Some(schema), PortSide::Input, Some(color)),
                Padding::symmetric(6.0, 0.0)
                    .child(Text::new(schema.label.to_string()).class("node-card-port-label-in")),
            ]
    })
}

fn compact_port_out(node_id: NodeId, schema: PortSchema) -> Box<dyn Widget> {
    let color = wire_color(schema.kind, false);
    Box::new(mgui! {
        Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Padding::symmetric(6.0, 0.0)
                    .child(Text::new(schema.label.to_string()).class("node-card-port-label-out")),
                PortDot::new(node_id, Some(schema), PortSide::Output, Some(color)),
            ]
    })
}

fn compact_layout(
    node: NodeInstance,
    meta: &'static NodeKindMeta,
    centered: bool,
) -> impl Widget {
    let inputs = meta.inputs.resolve(&node);
    let outputs = meta.outputs.resolve(&node);

    let body_widget: Box<dyn Widget> = if let Some(builder) = meta.body {
        builder(&node)
    } else {
        Box::new(DecoratedBox::new().class("node-card-body-empty"))
    };

    let inputs_col: Box<dyn Widget> = if inputs.is_empty() {
        Box::new(DecoratedBox::new().class("node-card-port-col-empty"))
    } else if centered {
        let rows: Vec<Box<dyn Widget>> = inputs
            .iter()
            .map(|p| compact_port_in(node.id, *p))
            .collect();
        Box::new(
            Column::new()
                .gap(4.0)
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .class("node-card-port-col node-card-port-col-in")
                .children(rows),
        )
    } else {
        let extra = meta.port_row_extra;
        let rows: Vec<Box<dyn Widget>> = inputs
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let dot = compact_port_in(node.id, *p);
                if let Some(extra_fn) = extra {
                    let extra_widget = extra_fn(&node, idx);
                    Box::new(
                        Row::new()
                            .gap(6.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(vec![dot, extra_widget]),
                    ) as Box<dyn Widget>
                } else {
                    dot
                }
            })
            .collect();
        Box::new(
            Column::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .class("node-card-port-col node-card-port-col-in")
                .children(rows),
        )
    };

    let outputs_col: Box<dyn Widget> = if outputs.is_empty() {
        Box::new(DecoratedBox::new().class("node-card-port-col-empty"))
    } else {
        let rows: Vec<Box<dyn Widget>> = outputs
            .iter()
            .map(|p| compact_port_out(node.id, *p))
            .collect();
        Box::new(
            Column::new()
                .gap(4.0)
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .class("node-card-port-col node-card-port-col-out")
                .children(rows),
        )
    };

    let row = Row::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![inputs_col, body_widget, outputs_col]);

    DecoratedBox::new().child(row).class("node-card-body")
}

/// Реактивная строка с текущим значением порта `(id, port)`. Подписывается
/// на `NodeEditorCtx.values` через `RwSignal::get()`. Значение —
/// `PortValue::Float` (для Number/Add/Output скаляров); для Audio/Empty
/// показываем заглушку.
fn value_display(id: NodeId, port: &'static str, css_class: &'static str) -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        use super::types::PortValue;
        let map = use_context::<NodeEditorCtx>().values.get();
        let pv = map.get(&(id, port)).cloned().unwrap_or(PortValue::Empty);
        let txt = match pv {
            PortValue::Float(v) => format!("{:.2}", v),
            PortValue::Audio(_) => "♪ audio".to_string(),
            PortValue::AudioStream(_) => "♪ live".to_string(),
            PortValue::Text(s) => {
                let first_line = s.lines().next().unwrap_or("").chars().take(24).collect::<String>();
                if first_line.is_empty() {
                    "abc empty".to_string()
                } else {
                    format!("abc {}", first_line)
                }
            }
            PortValue::Empty => "—".to_string(),
            PortValue::Data(_) => "▮ tensor".to_string(),
            PortValue::VideoStream(s) => format!("▶ {}×{}", s.width, s.height),
        };
        vec![Box::new(
            Padding::symmetric(NODE_PADDING_H, 0.0)
                .child(Text::new(txt).class(css_class)),
        ) as Box<dyn Widget>]
    }))
}

fn port_row(node_id: NodeId, in_port: Option<PortSchema>, out_port: Option<PortSchema>) -> Box<dyn Widget> {
    let in_label = in_port.map(|p| p.label.to_string()).unwrap_or_default();
    let out_label = out_port.map(|p| p.label.to_string()).unwrap_or_default();
    let in_color = in_port.map(|p| wire_color(p.kind, false));
    let out_color = out_port.map(|p| wire_color(p.kind, false));

    Box::new(mgui! {
        Row::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                PortDot::new(node_id, in_port, PortSide::Input, in_color),
                Padding::symmetric(6.0, 0.0).child(Text::new(in_label).class("node-card-port-label-in")),
                PortLabelSpacer,
                Padding::symmetric(6.0, 0.0).child(Text::new(out_label).class("node-card-port-label-out")),
                PortDot::new(node_id, out_port, PortSide::Output, out_color),
            ]
    })
}

fn field_row(schema: FieldSchema, fields: Arc<Mutex<std::collections::HashMap<&'static str, FieldValue>>>) -> Box<dyn Widget> {
    let label = schema.label.to_string();
    let name = schema.name;
    let map = fields.lock().unwrap();
    let value = map.get(name).cloned();
    drop(map);

    let field_widget: Box<dyn Widget> = match (schema.ty, value) {
        (FieldType::Text, Some(FieldValue::Text(sig))) => {
            let initial = sig.get_untracked();
            Box::new(
                TextField::new()
                    .placeholder(tr!("nodes.field.text_placeholder"))
                    .text(initial)
                    .on_change(move |s| sig.set(s.to_string()))
                    .class("node-input-text"),
            )
        }
        (FieldType::Float, Some(FieldValue::Float(sig))) => {
            let initial = sig.get_untracked();
            Box::new(
                Slider::new()
                    .range(-100.0, 100.0)
                    .value(initial)
                    .on_change(move |v| sig.set(v))
                    .class("node-input-slider"),
            )
        }
        (FieldType::Int, Some(FieldValue::Int(sig))) => {
            let initial = sig.get_untracked();
            Box::new(
                SpinBox::new()
                    .min(-1000.0)
                    .max(1000.0)
                    .step(1.0)
                    .decimal_places(0)
                    .value(initial as f64)
                    .on_change(move |v| sig.set(v as i32))
                    .width(80.0)
                    .class("node-input-spinbox"),
            )
        }
        (FieldType::Bool, Some(FieldValue::Bool(sig))) => {
            let initial = sig.get_untracked();
            if name == "bypass" {
                Box::new(
                    Toggle::with_state(initial)
                        .on_change(move |v| sig.set(v))
                        .class("node-input-toggle"),
                )
            } else {
                Box::new(
                    Checkbox::checked(initial)
                        .on_change(move |v| sig.set(v))
                        .class("node-input-checkbox"),
                )
            }
        }
        (FieldType::Color, Some(FieldValue::Color(sig))) => {
            let c0 = sig.get_untracked();
            let cv = ColorValue::from_color(c0);
            Box::new(
                ColorPicker::new()
                    .color(cv)
                    .width(120.0)
                    .on_change(move |cv| {
                        // ColorValue → Color: переводим u8 → f32 и нормируем альфу.
                        let c = Color::from_srgb(cv.r, cv.g, cv.b, (cv.a as f32) / 255.0);
                        sig.set(c);
                    })
                    .class("node-input-color"),
            )
        }
        (FieldType::Choice(opts), Some(FieldValue::Choice(sig))) => {
            let initial_idx = sig.get_untracked();
            let opts_vec: Vec<&'static str> = opts.iter().copied().collect();
            let initial_label = opts_vec.get(initial_idx).copied().unwrap_or("");
            let items: Vec<DropdownItem> = opts_vec.iter().map(|o| DropdownItem::simple(*o)).collect();
            let opts_for_cb = opts_vec.clone();
            Box::new(
                Dropdown::with_items(items)
                    .selected(initial_label)
                    .on_change(move |val| {
                        if let Some(idx) = opts_for_cb.iter().position(|o| *o == val) {
                            sig.set(idx);
                        }
                    })
                    .class("node-input-dropdown"),
            )
        }
        _ => Box::new(Text::new("?").class("node-card-field-error")),
    };

    let row = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![
            Box::new(Padding::symmetric(NODE_PADDING_H, 2.0).child(Text::new(label).class("node-card-field-label"))) as Box<dyn Widget>,
            Box::new(FieldFlexSpacer) as Box<dyn Widget>,
            Box::new(WrapInPadding(field_widget, NODE_PADDING_H, 2.0)) as Box<dyn Widget>,
        ]);
    Box::new(row)
}

/// Маленький helper-widget: оборачивает Box<dyn Widget> в Padding с заданными
/// отступами. Нужен потому что `Padding::child` принимает только IntoWidget,
/// а у нас уже-боксированный Widget.
struct WrapInPadding(Box<dyn Widget>, f32, f32);
impl Widget for WrapInPadding {
    fn create_element(&self) -> Box<dyn Element> {
        // Создаём Padding с маленьким inner Widget'ом — но child устанавливать
        // через child() мы не можем. Вместо этого сами реализуем Element.
        struct PadE {
            id: ElementId, bounds: Rect,
            l: f32, r: f32, t: f32, b: f32,
            child_id: Option<ElementId>,
            dirty: DirtyFlags,
            classes: Vec<String>,
            mss: MssFields,
        }
        impl Element for PadE {
            fn update(&mut self, _w: &dyn Widget, _ctx: &mut UpdateContext) {}
            fn layout(&mut self, c: Constraints) -> Size {
                let w = if c.max_width.is_finite() { c.max_width } else { 0.0 };
                let h = if c.max_height.is_finite() { c.max_height } else { 0.0 };
                self.bounds = Rect::new(self.bounds.origin, Size::new(w, h));
                Size::new(w, h)
            }
            fn layout_hint(&self) -> LayoutHint {
                LayoutHint::Padding { left: self.l, top: self.t, right: self.r, bottom: self.b }
            }
            fn build_display_list(&self, _l: &mut DisplayList, _c: Rect) {}
            fn handle_event(&mut self, _e: &Event, _c: &mut EventContext) -> EventResult { EventResult::Ignored }
            fn passthrough_hit_test(&self) -> bool { true }
            fn children(&self) -> &[ElementId] {
                static EMPTY: &[ElementId] = &[];
                match self.child_id { Some(ref i) => std::slice::from_ref(i), None => EMPTY }
            }
            fn bounds(&self) -> Rect { self.bounds }
            fn set_position(&mut self, p: Point) { self.bounds.origin = p; }
            fn mark_dirty(&mut self, f: DirtyFlags) { self.dirty |= f; }
            fn clear_dirty(&mut self, f: DirtyFlags) { self.dirty.remove(f); }
            fn is_dirty(&self, f: DirtyFlags) -> bool { self.dirty.contains(f) }
            fn id(&self) -> ElementId { self.id }
            fn set_id(&mut self, i: ElementId) { self.id = i; }
            fn mount(&mut self, _t: &mut ElementTree) {}
            fn element_type_name(&self) -> &str { "WrapInPadding" }
            fn set_classes(&mut self, c: Vec<String>) { self.classes = c; }
            fn get_classes(&self) -> &[String] { &self.classes }
            fn reset_mss_styles(&mut self) { self.mss.reset(); }
            fn mss(&self) -> Option<&MssFields> { Some(&self.mss) }
            fn apply_computed_style(&mut self, s: &ComputedStyle) { self.mss.apply(s); }
        }
        Box::new(PadE {
            id: ElementId::new(),
            bounds: Rect::zero(),
            l: self.1, r: self.1, t: self.2, b: self.2,
            child_id: None,
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            classes: Vec::new(),
            mss: MssFields::new(),
        })
    }
    fn can_update(&self, other: &dyn Any) -> bool { other.is::<Self>() }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn mount(&self, tree: &mut ElementTree, parent_id: ElementId) {
        let child_element = self.0.create_element();
        let child_id = tree.insert_with_type_id(child_element, Some(parent_id), self.0.as_any().type_id());
        self.0.mount(tree, child_id);
    }
    fn child_widgets(&self) -> Vec<&dyn Widget> { vec![self.0.as_ref()] }
}

// ── flex spacers (через MSS flex-grow) ──────────────────────────────────
// syngui поддерживает flex через MSS-класс `flex-grow: 1`. Использовать
// пустой DecoratedBox с классом, который определён в node_editor.mss.
struct PortLabelSpacer;
impl Widget for PortLabelSpacer {
    fn create_element(&self) -> Box<dyn Element> {
        DecoratedBox::new().class("node-card-port-spacer").create_element()
    }
    fn can_update(&self, other: &dyn Any) -> bool {
        other.is::<Self>() || other.is::<DecoratedBox>()
    }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn mount(&self, _tree: &mut ElementTree, _parent_id: ElementId) {}
}

struct FieldFlexSpacer;
impl Widget for FieldFlexSpacer {
    fn create_element(&self) -> Box<dyn Element> {
        DecoratedBox::new().class("node-card-field-spacer").create_element()
    }
    fn can_update(&self, other: &dyn Any) -> bool {
        other.is::<Self>() || other.is::<DecoratedBox>()
    }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn mount(&self, _tree: &mut ElementTree, _parent_id: ElementId) {}
}

// ───────────────────────────────────────────────────────────────────────
// DragHandle — кастомный element для drag header'а ноды.
// ───────────────────────────────────────────────────────────────────────

pub struct DragHandle {
    child: Option<Box<dyn Widget>>,
    pos_signal: RwSignal<Point>,
    height: f32,
}

impl DragHandle {
    fn new(pos_signal: RwSignal<Point>, height: f32, child: impl Widget + 'static) -> Self {
        Self {
            child: Some(Box::new(child)),
            pos_signal,
            height,
        }
    }
}

impl Widget for DragHandle {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(DragHandleElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            pos_signal: self.pos_signal,
            height: self.height,
            drag_start_world: None,
            drag_start_pos: Point::zero(),
            child_id: None,
            dirty_flags: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            classes: Vec::new(),
            mss: MssFields::new(),
        })
    }

    fn can_update(&self, other: &dyn Any) -> bool { other.is::<Self>() }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn mount(&self, tree: &mut ElementTree, parent_id: ElementId) {
        if let Some(child) = &self.child {
            let child_element = child.create_element();
            let child_id = tree.insert_with_type_id(child_element, Some(parent_id), child.as_any().type_id());
            child.mount(tree, child_id);
        }
    }

    fn child_widgets(&self) -> Vec<&dyn Widget> {
        self.child.as_ref().map(|c| vec![c.as_ref()]).unwrap_or_default()
    }
}

struct DragHandleElement {
    id: ElementId,
    bounds: Rect,
    pos_signal: RwSignal<Point>,
    height: f32,
    drag_start_world: Option<Point>,
    drag_start_pos: Point,
    child_id: Option<ElementId>,
    dirty_flags: DirtyFlags,
    classes: Vec<String>,
    mss: MssFields,
}

impl Element for DragHandleElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(w) = widget.as_any().downcast_ref::<DragHandle>() {
            self.pos_signal = w.pos_signal;
            self.height = w.height;
        }
    }

    fn layout(&mut self, constraints: Constraints) -> Size {
        // DragHandle растягивается по ширине родителя (= ширине body row карточки),
        // фиксирует только высоту HEADER_HEIGHT.
        let w = if constraints.max_width.is_finite() {
            constraints.max_width
        } else {
            0.0
        };
        self.bounds = Rect::new(self.bounds.origin, Size::new(w, self.height));
        Size::new(w, self.height)
    }

    fn explicit_dimensions(&self, _pw: f32, _ph: f32) -> (Option<f32>, Option<f32>) {
        // None по ширине — родительский Column возьмёт max(child.width)
        // и tight constraints передаст обратно в layout(...) через measure_padding.
        (None, Some(self.height))
    }

    fn layout_hint(&self) -> LayoutHint {
        LayoutHint::Padding { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 }
    }

    fn build_display_list(&self, _list: &mut DisplayList, _clip: Rect) {}

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        match event {
            Event::MouseDown { button: MouseButton::Left, position } if self.bounds.contains(*position) => {
                self.drag_start_world = Some(*position);
                self.drag_start_pos = self.pos_signal.get_untracked();
                ctx.set_cursor(CursorIcon::Grabbing);
                EventResult::Handled
            }
            Event::MouseMove(pos) => {
                if let Some(start) = self.drag_start_world {
                    let dx = pos.x - start.x;
                    let dy = pos.y - start.y;
                    let new_pos = Point::new(self.drag_start_pos.x + dx, self.drag_start_pos.y + dy);
                    self.pos_signal.set(new_pos);
                    ctx.set_cursor(CursorIcon::Grabbing);
                    return EventResult::Handled;
                }
                if self.bounds.contains(*pos) {
                    ctx.set_cursor(CursorIcon::Grab);
                }
                EventResult::Ignored
            }
            Event::MouseUp { button: MouseButton::Left, .. } if self.drag_start_world.is_some() => {
                self.drag_start_world = None;
                ctx.set_cursor(CursorIcon::Default);
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }

    fn children(&self) -> &[ElementId] {
        static EMPTY: &[ElementId] = &[];
        match self.child_id {
            Some(ref id) => std::slice::from_ref(id),
            None => EMPTY,
        }
    }

    fn bounds(&self) -> Rect { self.bounds }
    fn set_position(&mut self, pos: Point) { self.bounds.origin = pos; }
    fn mark_dirty(&mut self, f: DirtyFlags) { self.dirty_flags |= f; }
    fn clear_dirty(&mut self, f: DirtyFlags) { self.dirty_flags.remove(f); }
    fn is_dirty(&self, f: DirtyFlags) -> bool { self.dirty_flags.contains(f) }
    fn id(&self) -> ElementId { self.id }
    fn set_id(&mut self, id: ElementId) { self.id = id; }
    fn mount(&mut self, _tree: &mut ElementTree) {}
    fn element_type_name(&self) -> &str { "DragHandle" }
    fn set_classes(&mut self, c: Vec<String>) { self.classes = c; }
    fn get_classes(&self) -> &[String] { &self.classes }
    fn reset_mss_styles(&mut self) { self.mss.reset(); }
    fn mss(&self) -> Option<&MssFields> { Some(&self.mss) }
    fn apply_computed_style(&mut self, s: &ComputedStyle) { self.mss.apply(s); }
}

// ───────────────────────────────────────────────────────────────────────
// PortDot — Element для одного порта (input/output).
// ───────────────────────────────────────────────────────────────────────

pub struct PortDot {
    node_id: NodeId,
    port: Option<PortSchema>,
    side: PortSide,
    color: Option<Color>,
}

impl PortDot {
    fn new(node_id: NodeId, port: Option<PortSchema>, side: PortSide, color: Option<Color>) -> Self {
        Self { node_id, port, side, color }
    }
}

/// Helper для встраивания port-точки в кастомное body (audio-ноды).
/// Использует тот же `PortDot`, что и обычные port-row’ы — поэтому
/// hit-test, `set_position` со смещением на ±PORT_DOT/2 и регистрация
/// в overlay-стеке работают идентично.
pub fn inline_port_dot(
    node_id: NodeId,
    schema: PortSchema,
    side: PortSide,
) -> PortDot {
    let color = wire_color(schema.kind, false);
    PortDot::new(node_id, Some(schema), side, Some(color))
}

impl Widget for PortDot {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(PortDotElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            node_id: self.node_id,
            port: self.port,
            side: self.side,
            color: self.color.unwrap_or(Color::from_hex("#94A3B8")),
            dirty_flags: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            classes: Vec::new(),
            mss: MssFields::new(),
        })
    }
    fn can_update(&self, other: &dyn Any) -> bool { other.is::<Self>() }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn mount(&self, _tree: &mut ElementTree, _parent_id: ElementId) {}
}

struct PortDotElement {
    id: ElementId,
    bounds: Rect,
    node_id: NodeId,
    port: Option<PortSchema>,
    side: PortSide,
    color: Color,
    dirty_flags: DirtyFlags,
    classes: Vec<String>,
    mss: MssFields,
}

impl Element for PortDotElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(w) = widget.as_any().downcast_ref::<PortDot>() {
            self.node_id = w.node_id;
            self.port = w.port;
            self.side = w.side;
            if let Some(c) = w.color { self.color = c; }
            self.mark_dirty(DirtyFlags::RENDER);
        }
    }

    fn layout(&mut self, _c: Constraints) -> Size {
        let s = Size::new(PORT_DOT, PORT_ROW_HEIGHT);
        self.bounds = Rect::new(self.bounds.origin, s);
        s
    }

    fn explicit_dimensions(&self, _pw: f32, _ph: f32) -> (Option<f32>, Option<f32>) {
        (Some(PORT_DOT), Some(PORT_ROW_HEIGHT))
    }

    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        if self.port.is_none() { return; }
        let r = self.bounds;
        let cx = r.origin.x + r.size.width / 2.0;
        let cy = r.origin.y + r.size.height / 2.0;
        let radius = PORT_DOT * 0.5;
        let dot = Rect::new(
            Point::new(cx - radius, cy - radius),
            Size::new(PORT_DOT, PORT_DOT),
        );
        // Border темнее основного цвета визуально «отрывает» точку от
        // фона/тени карточки и не даёт ей выглядеть полупрозрачной.
        let border_color = self.color.darken(0.35);
        list.push_rect_bordered(dot, self.color, [radius; 4], Border::new(1.5, border_color));
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut EventContext) -> EventResult {
        let Some(port) = self.port else { return EventResult::Ignored; };
        match event {
            Event::MouseDown { button: MouseButton::Left, position } if self.bounds.contains(*position) => {
                let nctx = use_context::<NodeEditorCtx>();
                if self.side == PortSide::Output {
                    // Start-coord должен быть в canvas-local-системе (как и
                    // pending.current после `update_wire`). Используем
                    // `port_world_pos`-формулу — она работает в той же системе,
                    // что Stack кладёт ноды (`node.pos + offset_within_card`).
                    if let Some(node) = nctx.find_node(self.node_id) {
                        if let Some((world, _)) = port_world_pos(&node, self.side, port.name) {
                            nctx.start_wire(self.node_id, port.name, port.kind, world);
                            ctx.capture();
                            return EventResult::Handled;
                        }
                    }
                }
                EventResult::Ignored
            }
            // Right-click на PortDot отключает связь(и) этого порта. Bezier-
            // curve между точками сама не имеет hit-test, поэтому единственный
            // удобный «якорь» для удаления — port-точка с обеих сторон.
            Event::MouseDown { button: MouseButton::Right, position } if self.bounds.contains(*position) => {
                let nctx = use_context::<NodeEditorCtx>();
                let removed = nctx.disconnect_port(self.node_id, port.name, self.side);
                if removed > 0 {
                    return EventResult::Handled;
                }
                EventResult::Ignored
            }
            Event::MouseMove(pos) => {
                let nctx = use_context::<NodeEditorCtx>();
                if nctx.pending.get_untracked().is_some() {
                    nctx.update_wire(*pos);
                    return EventResult::Handled;
                }
                EventResult::Ignored
            }
            Event::MouseUp { button: MouseButton::Left, position } => {
                let nctx = use_context::<NodeEditorCtx>();
                if nctx.pending.get_untracked().is_some() {
                    // Захваченный output-port получает MouseUp первым (см.
                    // dispatch_positional → captor-first), значит ВСЕГДА
                    // ищем input-порт под cursor. На input-порте логика та же
                    // (если drag начался не отсюда — pending тоже выставлен).
                    if let Some((to_node, to_port)) = nctx.find_input_port_at(*position) {
                        nctx.complete_wire(to_node, to_port);
                    } else {
                        nctx.cancel_wire();
                    }
                    return EventResult::Handled;
                }
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }

    fn children(&self) -> &[ElementId] { &[] }
    fn bounds(&self) -> Rect { self.bounds }
    fn set_position(&mut self, pos: Point) {
        // Сдвигаем bounds так, чтобы центр точки оказался на границе карточки —
        // половина круга «выглядывает» наружу. Hit-test продолжает работать,
        // потому что круг рисуется в центре bounds, а bounds полностью
        // покрывает круг (только смещены).
        //
        // Координаты в port_positions НЕ пишем: bounds здесь — global-world
        // (после всех transform'ов от PanZoomViewport), а wires-Canvas рисует
        // в local-pre-transform. `port_world_pos` использует `node.pos +
        // node.size`-formula (оба signal'а в local-system) — это совпадает
        // с системой координат wires-Canvas.
        let dx = match self.side {
            PortSide::Input => -PORT_DOT * 0.5,
            PortSide::Output => PORT_DOT * 0.5,
        };
        self.bounds.origin = Point::new(pos.x + dx, pos.y);
    }
    fn overlay_request(&self) -> Option<(Rect, bool)> {
        // Половина точки выходит за границу карточки; hit-test_path обычной
        // DFS-веткой не доходит туда — bounds промежуточных контейнеров
        // (Row → Column → DecoratedBox → Positioned) ограничены NODE_WIDTH
        // и не содержат смещённую точку. Регистрация в overlay-стеке
        // поднимает PortDot в Phase 1 dispatch (event.rs:125), события
        // приходят сразу по bounds порта, минуя DFS.
        if self.port.is_some() {
            Some((self.bounds, false))
        } else {
            None
        }
    }
    fn mark_dirty(&mut self, f: DirtyFlags) { self.dirty_flags |= f; }
    fn clear_dirty(&mut self, f: DirtyFlags) { self.dirty_flags.remove(f); }
    fn is_dirty(&self, f: DirtyFlags) -> bool { self.dirty_flags.contains(f) }
    fn id(&self) -> ElementId { self.id }
    fn set_id(&mut self, id: ElementId) { self.id = id; }
    fn mount(&mut self, _tree: &mut ElementTree) {}
    fn element_type_name(&self) -> &str { "PortDot" }
    fn set_classes(&mut self, c: Vec<String>) { self.classes = c; }
    fn get_classes(&self) -> &[String] { &self.classes }
    fn reset_mss_styles(&mut self) { self.mss.reset(); }
    fn mss(&self) -> Option<&MssFields> { Some(&self.mss) }
    fn apply_computed_style(&mut self, s: &ComputedStyle) { self.mss.apply(s); }
}

// ───────────────────────────────────────────────────────────────────────
// SizeReport — прозрачный decorator, публикующий фактический layout-size
// child'а в `RwSignal<Size>`. CSS-аналог: `ResizeObserver` API в DOM.
// ───────────────────────────────────────────────────────────────────────

pub struct SizeReport {
    target: RwSignal<Rect>,
    child: Option<Box<dyn Widget>>,
}

impl SizeReport {
    fn new(target: RwSignal<Rect>, child: impl Widget + 'static) -> Self {
        Self { target, child: Some(Box::new(child)) }
    }
}

impl Widget for SizeReport {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(SizeReportElement {
            id: ElementId::new(),
            bounds: Rect::zero(),
            target: self.target,
            child_id: None,
            dirty_flags: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            classes: Vec::new(),
            mss: MssFields::new(),
        })
    }

    fn can_update(&self, other: &dyn Any) -> bool { other.is::<Self>() }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }

    fn mount(&self, tree: &mut ElementTree, parent_id: ElementId) {
        if let Some(child) = &self.child {
            let child_element = child.create_element();
            let child_id = tree.insert_with_type_id(child_element, Some(parent_id), child.as_any().type_id());
            child.mount(tree, child_id);
        }
    }

    fn child_widgets(&self) -> Vec<&dyn Widget> {
        self.child.as_ref().map(|c| vec![c.as_ref()]).unwrap_or_default()
    }
}

struct SizeReportElement {
    id: ElementId,
    bounds: Rect,
    target: RwSignal<Rect>,
    child_id: Option<ElementId>,
    dirty_flags: DirtyFlags,
    classes: Vec<String>,
    mss: MssFields,
}

impl Element for SizeReportElement {
    fn update(&mut self, widget: &dyn Widget, _ctx: &mut UpdateContext) {
        if let Some(w) = widget.as_any().downcast_ref::<SizeReport>() {
            self.target = w.target;
        }
    }

    fn layout(&mut self, constraints: Constraints) -> Size {
        let size = Size::new(constraints.max_width, constraints.max_height);
        self.bounds = Rect::new(self.bounds.origin, size);
        self.publish();
        size
    }

    fn layout_hint(&self) -> LayoutHint {
        LayoutHint::Padding { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 }
    }

    fn build_display_list(&self, _list: &mut DisplayList, _clip: Rect) {}
    fn handle_event(&mut self, _event: &Event, _ctx: &mut EventContext) -> EventResult {
        EventResult::Ignored
    }

    fn children(&self) -> &[ElementId] {
        static EMPTY: &[ElementId] = &[];
        match self.child_id {
            Some(ref id) => std::slice::from_ref(id),
            None => EMPTY,
        }
    }

    fn bounds(&self) -> Rect { self.bounds }
    fn set_position(&mut self, pos: Point) {
        self.bounds.origin = pos;
        self.publish();
    }
    fn mark_dirty(&mut self, f: DirtyFlags) { self.dirty_flags |= f; }
    fn clear_dirty(&mut self, f: DirtyFlags) { self.dirty_flags.remove(f); }
    fn is_dirty(&self, f: DirtyFlags) -> bool { self.dirty_flags.contains(f) }
    fn id(&self) -> ElementId { self.id }
    fn set_id(&mut self, id: ElementId) { self.id = id; }
    fn mount(&mut self, _tree: &mut ElementTree) {}
    fn element_type_name(&self) -> &str { "SizeReport" }
    fn set_classes(&mut self, c: Vec<String>) { self.classes = c; }
    fn get_classes(&self) -> &[String] { &self.classes }
    fn reset_mss_styles(&mut self) { self.mss.reset(); }
    fn mss(&self) -> Option<&MssFields> { Some(&self.mss) }
    fn apply_computed_style(&mut self, s: &ComputedStyle) { self.mss.apply(s); }
}

impl SizeReportElement {
    fn publish(&self) {
        let cur = self.target.get_untracked();
        if (cur.origin.x - self.bounds.origin.x).abs() > 0.01
            || (cur.origin.y - self.bounds.origin.y).abs() > 0.01
            || (cur.size.width - self.bounds.size.width).abs() > 0.01
            || (cur.size.height - self.bounds.size.height).abs() > 0.01
        {
            self.target.set(self.bounds);
        }
    }
}
