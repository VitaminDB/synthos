//! Глобальное состояние редактора нод (один на приложение).
//!
//! Все коллекции — `RwSignal`, чтобы UI реактивно подхватывал изменения.
//! pos каждой ноды — отдельный `RwSignal<Point>`, что позволяет drag'у
//! одной ноды не вызывать пересборку всего Stack'а.

use std::collections::HashMap;

use syngui::core::Point;
use syngui::prelude::*;

use super::eval;
use super::registry;
use super::types::*;

/// Контекст редактора нод. Кладётся в `provide_context` рядом с `AppCtx`.
#[derive(Clone, Copy)]
pub struct NodeEditorCtx {
    /// Список нод. Чтобы добавление/удаление вызывало пересборку Stack —
    /// храним `Vec<NodeInstance>` целиком в RwSignal.
    pub nodes: RwSignal<Vec<NodeInstance>>,
    /// Соединения. Изменение списка вызывает перерисовку Canvas wires.
    pub connections: RwSignal<Vec<Connection>>,
    /// Текущая позиция «камеры» — пан в local-coords PanZoomViewport'а.
    pub pan: RwSignal<Point>,
    /// Зум, [min_zoom, max_zoom] из PanZoomViewport.
    pub zoom: RwSignal<f32>,
    /// id выделенной ноды (или None).
    pub selected: RwSignal<Option<NodeId>>,
    /// Provisional wire — drag-from-output.
    pub pending: RwSignal<Option<PendingWire>>,
    /// Открыто ли контекстное меню фона.
    pub menu_open: RwSignal<bool>,
    /// World-pos места клика — используется для размещения новой ноды
    /// «под курсором» с учётом pan/zoom.
    pub menu_world_pos: RwSignal<Point>,
    /// Screen-pos места клика — позиция overlay-меню в экранных координатах,
    /// независимая от pan/zoom. Передаётся в `PopupMenu.position()`.
    pub menu_screen_pos: RwSignal<Point>,
    /// Auto-increment id для новых нод.
    pub next_id: RwSignal<u64>,
    /// Кэш вычисленных значений на портах: `(node_id, port_name) → PortValue`.
    /// Заполняется глобальным `create_effect` при изменении nodes/connections
    /// или любого Number.value-сигнала. Output-нода читает значение по
    /// pseudo-ключу `(id, "in")`. См. `eval::evaluate_graph`.
    ///
    /// `PortValue` — typed enum (Empty/Float/Audio): UI ноды читает нужный
    /// payload (`as_float()` или `as_audio()`).
    pub values: RwSignal<HashMap<(NodeId, &'static str), PortValue>>,
    /// id ноды, для которой открыт Portal-диалог цветового оттенка
    /// (ColorPicker). `None` — диалог закрыт. Открывается из ContextMenu
    /// карточки, см. `style_dialog`.
    pub tint_dialog: RwSignal<Option<NodeId>>,
}

impl NodeEditorCtx {
    pub fn new() -> Self {
        let ctx = Self {
            nodes: use_signal(Vec::<NodeInstance>::new()),
            connections: use_signal(Vec::<Connection>::new()),
            pan: use_signal(Point::new(40.0, 40.0)),
            zoom: use_signal(1.0_f32),
            selected: use_signal(None::<NodeId>),
            pending: use_signal(None::<PendingWire>),
            menu_open: use_signal(false),
            menu_world_pos: use_signal(Point::zero()),
            menu_screen_pos: use_signal(Point::zero()),
            next_id: use_signal(1_u64),
            values: use_signal(HashMap::<(NodeId, &'static str), PortValue>::new()),
            tint_dialog: use_signal(None::<NodeId>),
        };
        ctx.add_node(NodeKind::Number, Point::new(80.0, 80.0));

        // Реактивный пересчёт значений на портах. `evaluate_graph(track=true)`
        // читает Vec нод, connections и Float-сигналы Number-нод через
        // `RwSignal::get()`, что подписывает effect на любой источник
        // изменений. Mutex<HashMap> внутри NodeInstance.fields реактивен не
        // напрямую, но HashMap-ключи стабильны: при добавлении/удалении ноды
        // меняется Vec и effect перезапустится — переподпишется на новые
        // FieldValue::Float сигналы.
        let values = ctx.values;
        let nodes_sig = ctx.nodes;
        let conns_sig = ctx.connections;
        create_effect(move || {
            let nodes = nodes_sig.get();
            let conns = conns_sig.get();
            let map = eval::evaluate_graph(&nodes, &conns, true);
            values.set(map);
        });

        ctx.install_node_timers();

        ctx
    }

    /// Секундомеры нод. Эффект подписан на `busy_signal` каждой ноды графа:
    /// флип `false → true` запускает отсчёт, обратный — фиксирует
    /// длительность в `timing.last_ms`. Точка наблюдения выбрана именно
    /// здесь (а не в `run_controls`), потому что busy флипают и per-node
    /// Play-кнопки, и эффект должен жить вместе с вкладкой, а не с её
    /// отрисованным canvas'ом — иначе таймеры замирали бы на неактивной
    /// вкладке.
    ///
    /// Свои `timing`-сигналы эффект читает untracked — иначе `start()`
    /// перезапускал бы сам эффект.
    fn install_node_timers(&self) {
        let nodes_sig = self.nodes;
        create_effect(move || {
            let nodes = nodes_sig.get();
            for n in &nodes {
                let Some(hook) = registry::meta(n.kind).busy_signal else {
                    continue;
                };
                let Some(busy_sig) = hook(n) else { continue };
                let busy = busy_sig.get();
                let running = n.timing.is_running_untracked();
                if busy && !running {
                    n.timing.start();
                } else if !busy && running {
                    n.timing.finish();
                }
            }
        });
    }

    /// Добавить ноду указанного типа в указанной world-позиции.
    pub fn add_node(&self, kind: NodeKind, world_pos: Point) -> NodeId {
        let id_n = self.next_id.get_untracked();
        self.next_id.set(id_n + 1);
        let id = NodeId(id_n);
        let inst = NodeInstance {
            id,
            kind,
            pos: use_signal(world_pos),
            bounds: use_signal(syngui::core::Rect::zero()),
            fields: registry::default_fields(kind),
            runtime: registry::default_runtime(kind),
            style: use_signal(crate::pages::node_editor::types::NodeStyle::default()),
            enabled: use_signal(true),
            timing: super::timing::Stopwatch::new(),
        };
        let mut v = self.nodes.get_untracked();
        v.push(inst);
        self.nodes.set(v);
        id
    }

    /// Установить количество активных входов у Mixer-ноды + удалить все
    /// connections к портам `in_k` где `k > n`. Used by «Add Node» меню
    /// при выборе пресета (2/3/4/5/Custom). No-op для не-Mixer.
    pub fn set_mixer_n_inputs(&self, id: NodeId, n: usize) {
        let n = n.clamp(2, crate::pages::node_editor::types::MIXER_MAX_INPUTS);
        let nodes = self.nodes.get_untracked();
        let Some(node) = nodes.iter().find(|x| x.id == id) else { return };
        if !matches!(node.kind, NodeKind::Mixer) {
            return;
        }
        if let Ok(mut g) = node.runtime.lock() {
            if let crate::pages::node_editor::types::NodeRuntime::Mixer { n_inputs, .. } = &mut *g {
                if n_inputs.get_untracked() != n {
                    n_inputs.set(n);
                }
            }
        }
        // Чистим connections к input-портам, которых больше нет в активном
        // наборе (in_{n+1} .. in_16). Имена статические — собираем отбрасываемые.
        let mut conns = self.connections.get_untracked();
        let initial_len = conns.len();
        conns.retain(|c| {
            if c.to_node != id {
                return true;
            }
            // active имена — in_1 .. in_n; всё, что выше, обрезаем.
            for i in n..crate::pages::node_editor::types::MIXER_MAX_INPUTS {
                if c.to_port == crate::pages::node_editor::nodes::audio_mixer::MIXER_PORT_SCHEMAS_FULL[i].name
                {
                    return false;
                }
            }
            true
        });
        if conns.len() != initial_len {
            self.connections.set(conns);
        }
    }

    /// Удалить ноду + все её связи.
    pub fn remove_node(&self, id: NodeId) {
        let mut v = self.nodes.get_untracked();
        v.retain(|n| n.id != id);
        self.nodes.set(v);
        let mut c = self.connections.get_untracked();
        c.retain(|c| c.from_node != id && c.to_node != id);
        self.connections.set(c);
        if self.selected.get_untracked() == Some(id) {
            self.selected.set(None);
        }
    }

    /// Дублировать ноду (новый id, сдвиг (24, 24)). Стиль и enabled
    /// копируются с оригинала — пользователь ожидает «такая же, но рядом».
    pub fn duplicate_node(&self, id: NodeId) {
        let v = self.nodes.get_untracked();
        if let Some(orig) = v.iter().find(|n| n.id == id).cloned() {
            let pos = orig.pos.get_untracked();
            let new_id = self.add_node(orig.kind, Point::new(pos.x + 24.0, pos.y + 24.0));
            // Перенести стиль и enabled на новую ноду.
            let nodes = self.nodes.get_untracked();
            if let Some(new_node) = nodes.iter().find(|n| n.id == new_id) {
                new_node.style.set(orig.style.get_untracked());
                new_node.enabled.set(orig.enabled.get_untracked());
            }
        }
    }

    /// Найти ноду по id (не subscribe).
    pub fn find_node(&self, id: NodeId) -> Option<NodeInstance> {
        self.nodes
            .get_untracked()
            .into_iter()
            .find(|n| n.id == id)
    }

    /// Сместить координату из position-pass-coord (= canvas.bounds.origin +
    /// canvas-local) в canvas-local. Canvas-local = node.pos + offset_within_card,
    /// в той же системе, что Stack кладёт свои Positioned-children. wires-Canvas
    /// рисует bezier'ы интерпретируя coords как canvas-local — поэтому все
    /// данные, передаваемые в pending.current, должны быть в этой системе.
    ///
    /// canvas_origin = `node.bounds.origin - node.pos` для любой существующей
    /// ноды (Stack кладёт ноду в `Stack.origin + node.pos`, поэтому
    /// `bounds.origin = Stack.origin + node.pos` ⇒ `Stack.origin = bounds.origin - node.pos`).
    fn to_canvas_local(&self, world: Point) -> Point {
        let nodes = self.nodes.get_untracked();
        if let Some(n) = nodes.first() {
            let b = n.bounds.get_untracked();
            let p = n.pos.get_untracked();
            let canvas_origin_x = b.origin.x - p.x;
            let canvas_origin_y = b.origin.y - p.y;
            Point::new(world.x - canvas_origin_x, world.y - canvas_origin_y)
        } else {
            world
        }
    }

    /// Начать drag нового провода с output-порта. `world` — в canvas-local-coord
    /// (вычисляется из `port_world_pos`).
    pub fn start_wire(&self, from_node: NodeId, from_port: &'static str, kind: PortKind, world: Point) {
        self.pending.set(Some(PendingWire {
            from_node,
            from_port,
            from_kind: kind,
            current: world,
        }));
    }

    /// Обновить конец pending-провода. `world` приходит в position-pass-coord
    /// (после inverse-transform от PanZoom), здесь конвертируется в canvas-local.
    pub fn update_wire(&self, world: Point) {
        let local = self.to_canvas_local(world);
        if let Some(mut p) = self.pending.get_untracked() {
            p.current = local;
            self.pending.set(Some(p));
        }
    }

    /// Найти input-порт под `world`-координатой (с допуском). Возвращает
    /// `(node_id, port_name)`. Используется при MouseUp drag-wire, когда
    /// событие приходит к захваченному output-порту, а курсор находится над
    /// каким-то input-портом другой ноды (возможно, не над самим dot, а
    /// рядом — допуск делает соединение «магнитным»).
    pub fn find_input_port_at(&self, world: Point) -> Option<(NodeId, &'static str)> {
        const TOLERANCE_SQ: f32 = 14.0 * 14.0;
        // `world` приходит от MouseUp handler в position-pass-coord (после
        // inverse-transform от PanZoom). `port_world_pos` возвращает
        // canvas-local — приводим world к ней для корректного distance-check.
        let world = self.to_canvas_local(world);
        let nodes = self.nodes.get_untracked();
        for node in &nodes {
            let meta = registry::meta(node.kind);
            for p in meta.inputs.resolve(node) {
                if let Some((center, _)) =
                    super::node_view::port_world_pos(node, PortSide::Input, p.name)
                {
                    let dx = center.x - world.x;
                    let dy = center.y - world.y;
                    if dx * dx + dy * dy <= TOLERANCE_SQ {
                        return Some((node.id, p.name));
                    }
                }
            }
        }
        None
    }

    /// Завершить drag: если to_node/to_port валидны — добавить connection.
    pub fn complete_wire(&self, to_node: NodeId, to_port: &'static str) {
        if let Some(p) = self.pending.get_untracked() {
            if p.from_node != to_node {
                let conn = Connection {
                    from_node: p.from_node,
                    from_port: p.from_port,
                    to_node,
                    to_port,
                };
                let mut v = self.connections.get_untracked();
                if !v.iter().any(|c| *c == conn) {
                    v.push(conn);
                    self.connections.set(v);
                }
            }
        }
        self.pending.set(None);
    }

    /// Отменить pending без добавления.
    pub fn cancel_wire(&self) {
        self.pending.set(None);
    }

    /// Удалить связь по индексу.
    pub fn delete_connection(&self, idx: usize) {
        let mut v = self.connections.get_untracked();
        if idx < v.len() {
            v.remove(idx);
            self.connections.set(v);
        }
    }

    /// Отключить все связи указанной ноды.
    pub fn disconnect_all(&self, id: NodeId) {
        let mut c = self.connections.get_untracked();
        c.retain(|c| c.from_node != id && c.to_node != id);
        self.connections.set(c);
    }

    /// Отключить связи к/от конкретного порта. Если `side == Input` — удаляет
    /// единственную connection где `to_node/to_port == (id, port)`. Если
    /// `side == Output` — удаляет все исходящие связи с этого output'а.
    /// Возвращает количество удалённых связей.
    pub fn disconnect_port(&self, id: NodeId, port: &'static str, side: PortSide) -> usize {
        let before = self.connections.get_untracked();
        let mut after = before.clone();
        match side {
            PortSide::Input => after.retain(|c| !(c.to_node == id && c.to_port == port)),
            PortSide::Output => after.retain(|c| !(c.from_node == id && c.from_port == port)),
        }
        let removed = before.len().saturating_sub(after.len());
        if removed > 0 {
            self.connections.set(after);
        }
        removed
    }

    /// Открыть контекстное меню фона. `world` — координата для размещения
    /// будущей ноды (после inverse pan/zoom), `screen` — позиция overlay-меню.
    pub fn open_bg_menu(&self, world: Point, screen: Point) {
        self.menu_world_pos.set(world);
        self.menu_screen_pos.set(screen);
        self.menu_open.set(true);
    }

    /// Сбросить зум/пан.
    pub fn reset_view(&self) {
        self.pan.set(Point::new(40.0, 40.0));
        self.zoom.set(1.0);
    }
}
