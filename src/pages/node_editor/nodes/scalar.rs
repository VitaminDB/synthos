//! Executors скалярных и passthrough-нод.
//!
//! Это семейство нод не имеет body builder'а: тело рисуется стандартным
//! `node_view` (field-rows + pseudo-port-rows). Все executor'ы — ZST,
//! регистрируются как `&'static dyn NodeExecutor` в `NodeKindMeta`.

use crate::pages::node_editor::eval::{EvalContext, NodeExecutor};
use crate::pages::node_editor::types::PortValue;

/// Number — источник константы. Поле `value: Float` → output `out: Float`.
pub struct NumberExec;

impl NodeExecutor for NumberExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let v = ctx.read_float_field("value");
        ctx.write_output("out", PortValue::Float(v));
    }
}

/// Add — суммирует input-порты `a` и `b`, выдаёт `out`.
pub struct AddExec;

impl NodeExecutor for AddExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let a = ctx.read_input("a").as_float();
        let b = ctx.read_input("b").as_float();
        ctx.write_output("out", PortValue::Float(a + b));
    }
}

/// Output — sink, копирует приходящее значение в pseudo-output `(id, "in")`,
/// чтобы UI мог его отобразить.
pub struct OutputExec;

impl NodeExecutor for OutputExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let v = ctx.read_input("in");
        ctx.write_output("in", v);
    }
}

