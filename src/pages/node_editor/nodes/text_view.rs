//! Body builder + executor для NodeKind::TextView — editable text display.
//!
//! Text in → MultilineTextEdit (editable) → Text out (passthrough).
//!
//! Логика:
//! - `evaluate` читает `PortValue::Text(s)` с input "in". Если строка
//!   отличается от закэшированной `last_input` — перезаписывает
//!   `output_text`, бампает `text_version` (Reactive в body rebuild'ит
//!   `MultilineTextEdit` с новым initial-text). Ручные правки пользователя
//!   не трогают `last_input` → повторный evaluate с тем же значением их
//!   не затрёт.
//! - Output port "out" всегда отражает текущее `output_text` (правки уходят
//!   downstream).
//!
//! Размер тела ноды управляется через `TransformBox` (паттерн
//! `MarkdownView`): `size` хранит текущие width/height, двусторонне
//! синхронизирован с drag-resize. `resize_mode` управляет видимостью
//! handle'ов (toggle в ContextMenu). Default 280×360 — вертикалка под
//! одну колонку транскрипта.

use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::input::MultilineTextEdit;
use syngui::widgets::{Reactive, TransformBox};

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::types::{NodeInstance, NodeRuntime, PortValue};

/// Executor: passthrough Text. Перезаписывает локальный `output_text` при
/// смене upstream, пишет текущее значение в output port.
pub struct TextViewExec;

impl NodeExecutor for TextViewExec {
    fn evaluate(&self, ctx: &mut EvalContext<'_>) {
        let in_pv = ctx.read_input("in");
        let track = ctx.track;

        // Снимаем Copy-сигналы + Arc-clone из runtime, чтобы не держать
        // его lock в момент ctx.write_output (тот требует &mut ctx).
        let extracted = match ctx.runtime().lock() {
            Ok(g) => match &*g {
                NodeRuntime::TextView {
                    output_text,
                    text_version,
                    last_input,
                    ..
                } => Some((*output_text, *text_version, last_input.clone())),
                _ => None,
            },
            Err(_) => None,
        };
        let Some((output_text, text_version, last_input)) = extracted else {
            ctx.write_output("out", PortValue::Empty);
            return;
        };

        let new_text: Option<String> = match &in_pv {
            PortValue::Text(s) => Some(s.clone()),
            PortValue::Empty => None,
            // Любой не-Text payload (Audio/Float/...) трактуем как
            // «нет текста» — не затираем существующий output_text.
            _ => None,
        };
        if let Ok(mut last) = last_input.lock() {
            let changed = match (&*last, &new_text) {
                (Some(a), Some(b)) => a != b,
                (None, Some(_)) => true,
                _ => false,
            };
            if changed {
                *last = new_text.clone();
                drop(last);
                if let Some(t) = new_text {
                    output_text.set(t);
                    text_version.update(|v| *v = v.wrapping_add(1));
                }
            }
        }

        let cur = if track {
            output_text.get()
        } else {
            output_text.get_untracked()
        };
        ctx.write_output("out", PortValue::Text(cur));
    }
}

pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let runtime = node.runtime.clone();

    let (output_text, text_version, resize_mode, size) = match runtime.lock() {
        Ok(g) => match &*g {
            NodeRuntime::TextView {
                output_text,
                text_version,
                resize_mode,
                size,
                ..
            } => (*output_text, *text_version, *resize_mode, *size),
            _ => return error_widget(tr!("nodes.common.invalid_runtime", name = "TextView")),
        },
        Err(_) => return error_widget("TextView: lock error"),
    };

    // Reactive rebuild при bump'е text_version — на user-edit version не
    // меняется → курсор не прыгает. `soft_wrap(true)` без `auto_height`
    // заставляет editor заполнять constraints от родителя (TransformBox),
    // поэтому высота карточки берётся из `size.height`.
    let editor = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _v = text_version.get();
        let initial = output_text.get_untracked();
        vec![Box::new(
            MultilineTextEdit::new()
                .text(initial)
                .placeholder(tr!("node.text_view.placeholder"))
                .soft_wrap(true)
                .on_change(move |s| output_text.set(s.to_string()))
                .class("node-input-text text-view-editor"),
        )]
    });

    let initial = size.get_untracked();
    Box::new(
        TransformBox::new()
            .resizable(true)
            .moveable(false)
            .rotatable(false)
            .active(resize_mode)
            .size_signal(size)
            .initial_size(initial.width, initial.height)
            .min_size(200.0, 160.0)
            .child(editor),
    )
}

/// Переключить видимость resize-handle'ов `TransformBox`'а.
/// Вызывается из ContextMenu по правому клику.
pub fn toggle_resize_mode(node: &NodeInstance) {
    let rt = node.runtime.lock().unwrap_or_else(|e| e.into_inner());
    if let NodeRuntime::TextView { resize_mode, .. } = &*rt {
        let cur = resize_mode.get_untracked();
        resize_mode.set(!cur);
    }
}

fn error_widget(msg: impl Into<String>) -> Box<dyn Widget> {
    Box::new(Padding::symmetric(10.0, 6.0).child(Text::new(msg).class("node-card-field-error")))
}
