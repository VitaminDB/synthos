//! Вычисление значений на портах графа нод.
//!
//! `evaluate_graph` — чистая функция: топологически сортирует ноды по
//! `Connection`-рёбрам (Kahn's algorithm) и для каждой ноды вычисляет её
//! outputs из inputs предшественников. Возвращает карту
//! `(NodeId, port_name) → PortValue` где хранятся как реальные outputs, так и
//! «pseudo-output» Output-ноды по ключу `(id, "in")` — туда записывается
//! приходящее на вход значение, чтобы UI мог его показать.
//!
//! `PortValue` поддерживает Float и Audio (см. `types::PortValue`) — это
//! позволяет одному pipeline нести и скаляры (Number/Add/Output) и аудио
//! (AudioFile/AudioRecorder/AudioPlayer).
//!
//! Параметр `track`:
//! - `true` — внутри читаем `RwSignal::get()`, что в реактивном контексте
//!   подписывает enclosing-effect на изменение полей. Используется в
//!   глобальном `create_effect` в `state.rs`.
//! - `false` — читаем `get_untracked()`. Полезно для одноразового вызова
//!   (тесты, debug).
//!
//! При обнаружении цикла (узлы, до которых не дошёл Kahn) их outputs
//! получают значение по умолчанию `Empty`. Это безопасный fallback —
//! редактор не падает.
//!
//! Сложность O(N + E) для топосорта, плюс O(N) на итерацию полей.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use syngui::core::sync::Mutex;

use super::types::{Connection, FieldValue, NodeId, NodeInstance, NodeRuntime, PortValue};

/// Ключ значения порта: (id ноды, имя порта).
pub type PortKey = (NodeId, &'static str);

/// Логика evaluate'а конкретного типа ноды. Каждая нода предоставляет свой
/// ZST-executor, описывающий как из inputs/полей/runtime получить outputs.
///
/// Object-safe trait: хранится как `&'static dyn NodeExecutor` в
/// `NodeKindMeta::executor`. Все встроенные executor'ы — stateless ZST'ы
/// (NumberExec/AddExec/AudioFileExec/…), поэтому `'static` тривиально
/// удовлетворён через `static FOO: FooExec = FooExec;`.
pub trait NodeExecutor: Send + Sync + 'static {
    /// Записать outputs ноды (через `ctx.write_output`) на основе inputs
    /// (через `ctx.read_input`), полей (`ctx.read_float_field`) и runtime
    /// (`ctx.runtime()`). Side-effects (UI-state updates через RwSignal)
    /// допустимы — eval-pipeline ожидает их и не реентерабелен.
    fn evaluate(&self, ctx: &mut EvalContext<'_>);
}

/// Контекст одного evaluate-вызова. Связывает текущий узел, входящие
/// связи и общую карту значений в режиме `track` (с подпиской reactivity).
pub struct EvalContext<'a> {
    /// Узел, для которого вычисляются outputs.
    pub node: &'a NodeInstance,
    /// Если `true` — чтения через `RwSignal::get()` (подписывают enclosing
    /// `create_effect`). Если `false` — через `get_untracked()`.
    pub track: bool,
    incoming: &'a HashMap<PortKey, PortKey>,
    values: &'a mut HashMap<PortKey, PortValue>,
}

impl<'a> EvalContext<'a> {
    /// Сконструировать контекст. Используется ядром `evaluate_graph`
    /// и unit-тестами executor'ов.
    pub fn new(
        node: &'a NodeInstance,
        incoming: &'a HashMap<PortKey, PortKey>,
        values: &'a mut HashMap<PortKey, PortValue>,
        track: bool,
    ) -> Self {
        Self { node, track, incoming, values }
    }

    /// id текущего узла.
    pub fn node_id(&self) -> NodeId {
        self.node.id
    }

    /// Доступ к long-lived runtime текущего узла. Executor сам решает,
    /// нужно ли `.lock()` (короткий) и как обрабатывать Err.
    pub fn runtime(&self) -> &Arc<Mutex<NodeRuntime>> {
        &self.node.runtime
    }

    /// Прочитать значение, приходящее на input-порт `port` текущей ноды.
    /// Если связи нет — `PortValue::Empty`.
    pub fn read_input(&self, port: &'static str) -> PortValue {
        let target = (self.node.id, port);
        self.incoming
            .get(&target)
            .and_then(|src| self.values.get(src).cloned())
            .unwrap_or(PortValue::Empty)
    }

    /// Записать output-порт `port` текущей ноды.
    pub fn write_output(&mut self, port: &'static str, v: PortValue) {
        self.values.insert((self.node.id, port), v);
    }

    /// Прочитать `Float`-поле ноды по имени. Не-Float / отсутствующее → 0.0.
    /// Чтение учитывает `self.track` (для reactivity).
    pub fn read_float_field(&self, name: &str) -> f32 {
        let map = self.node.fields.lock().unwrap();
        match map.get(name) {
            Some(FieldValue::Float(sig)) => {
                if self.track { sig.get() } else { sig.get_untracked() }
            }
            _ => 0.0,
        }
    }
}

/// Чистая функция вычисления графа.
///
/// Возвращает карту значений всех outputs (и pseudo-`in` для Output-нод).
pub fn evaluate_graph(
    nodes: &[NodeInstance],
    conns: &[Connection],
    track: bool,
) -> HashMap<PortKey, PortValue> {
    let mut values: HashMap<PortKey, PortValue> = HashMap::new();

    let mut incoming: HashMap<PortKey, PortKey> = HashMap::new();
    let mut out_edges: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let mut in_deg: HashMap<NodeId, usize> = HashMap::new();

    for n in nodes {
        in_deg.insert(n.id, 0);
        out_edges.insert(n.id, Vec::new());
    }
    let mut counted_pairs: std::collections::HashSet<(NodeId, NodeId)> =
        std::collections::HashSet::new();
    for c in conns {
        incoming.insert((c.to_node, c.to_port), (c.from_node, c.from_port));
        if counted_pairs.insert((c.from_node, c.to_node)) {
            *in_deg.entry(c.to_node).or_insert(0) += 1;
            out_edges.entry(c.from_node).or_default().push(c.to_node);
        }
    }

    let mut queue: VecDeque<NodeId> = VecDeque::new();
    for (id, deg) in &in_deg {
        if *deg == 0 {
            queue.push_back(*id);
        }
    }

    let nodes_by_id: HashMap<NodeId, &NodeInstance> =
        nodes.iter().map(|n| (n.id, n)).collect();

    while let Some(id) = queue.pop_front() {
        let Some(node) = nodes_by_id.get(&id) else { continue };
        // Gate disabled-нод: executor пропускается, outputs остаются
        // незаписанными → downstream получит `PortValue::Empty` при
        // `read_input`. Топосорт не нарушается — продолжаем decrement
        // in-degree, чтобы потомки тоже обработались.
        let enabled = if track { node.enabled.get() } else { node.enabled.get_untracked() };
        if enabled {
            let mut ctx = EvalContext::new(node, &incoming, &mut values, track);
            super::registry::meta(node.kind).executor.evaluate(&mut ctx);
        }

        if let Some(targets) = out_edges.get(&id) {
            for tid in targets {
                if let Some(d) = in_deg.get_mut(tid) {
                    if *d > 0 { *d -= 1; }
                    if *d == 0 {
                        queue.push_back(*tid);
                    }
                }
            }
        }
    }

    values
}


#[cfg(test)]
mod tests {
    use super::*;
    use super::super::registry;
    use super::super::types::NodeKind;
    use syngui::core::Point;
    use syngui::prelude::*;

    fn ensure_runtime() {}

    fn make_inst(kind: NodeKind, id: u64) -> NodeInstance {
        NodeInstance {
            id: NodeId(id),
            kind,
            pos: use_signal(Point::zero()),
            bounds: use_signal(syngui::core::Rect::zero()),
            fields: registry::default_fields(kind),
            runtime: registry::default_runtime(kind),
            style: use_signal(crate::pages::node_editor::types::NodeStyle::default()),
            enabled: use_signal(true),
            timing: crate::pages::node_editor::timing::Stopwatch::new(),
        }
    }

    #[test]
    fn float_pipeline_unchanged() {
        ensure_runtime();
        let n = make_inst(NodeKind::Number, 1);
        if let FieldValue::Float(s) = n.fields.lock().unwrap().get("value").cloned().unwrap() {
            s.set(3.5_f32);
        }
        let out = make_inst(NodeKind::Output, 2);
        let conns = vec![Connection {
            from_node: NodeId(1),
            from_port: "out",
            to_node: NodeId(2),
            to_port: "in",
        }];
        let map = evaluate_graph(&[n.clone(), out.clone()], &conns, false);
        let v = map.get(&(NodeId(2), "in")).cloned().unwrap_or(PortValue::Empty);
        assert!(
            (v.as_float() - 3.5).abs() < 1e-6,
            "expected 3.5 got {:?}",
            v
        );
    }

    #[test]
    fn audio_file_publishes_buffer() {
        use std::sync::Arc;
        use syngui::audio::AudioBuffer;
        ensure_runtime();
        let f = make_inst(NodeKind::AudioFile, 1);
        let buf = Arc::new(AudioBuffer::new(
            Arc::from(vec![0.1, 0.2, 0.3].into_boxed_slice()),
            48000,
            1,
        ));
        if let Ok(g) = f.runtime.lock() {
            if let NodeRuntime::AudioFile { buffer, .. } = &*g {
                buffer.set(Some(buf.clone()));
            }
        }
        let map = evaluate_graph(std::slice::from_ref(&f), &[], false);
        let pv = map.get(&(NodeId(1), "out")).cloned().unwrap();
        match pv {
            PortValue::Audio(b) => assert!(Arc::ptr_eq(&b, &buf)),
            other => panic!("expected Audio, got {other:?}"),
        }
    }
}
