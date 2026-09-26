//! Body builder для декоративной markdown-ноды node-editor'а.
//!
//! Нода без портов: содержимое — `MarkdownView` (Preview) либо
//! `MarkdownEditor` (Edit). Переключение между режимами и видимость
//! resize-handle'ов управляются `NodeRuntime::MarkdownView` сигналами
//! `edit_mode` / `resize_mode`, которые тоггл-функции переключают
//! из ContextMenu по правому клику.
//!
//! Resize-логика делегирована готовому `TransformBox` (8 handle'ов
//! на гранях/углах, корректные курсоры, MSS-кастомизация через `--tb-*`
//! переменные). `moveable(false)` — drag всей ноды остаётся на стандартном
//! `DragHandle` в header'е карточки; `rotatable(false)` — ротация не нужна.

use syngui::prelude::*;
use syngui::widget::WidgetExt;
use syngui::widgets::{Reactive, TransformBox};
use syngui::widgets::visual::markdown_editor::{EditorMode, MarkdownEditor};
use syngui::widgets::visual::markdown_view::MarkdownView;

use super::super::eval::{EvalContext, NodeExecutor};
use super::super::types::{NodeInstance, NodeRuntime};

/// No-op executor: декоративная нода без входов/выходов.
pub struct MarkdownViewExec;

impl NodeExecutor for MarkdownViewExec {
    fn evaluate(&self, _ctx: &mut EvalContext<'_>) {}
}

/// Переключить режим Preview ↔ Edit. Вызывается из ContextMenu.
pub fn toggle_edit_mode(node: &NodeInstance) {
    let rt = node.runtime.lock().unwrap_or_else(|e| e.into_inner());
    if let NodeRuntime::MarkdownView { edit_mode, .. } = &*rt {
        let cur = edit_mode.get_untracked();
        edit_mode.set(!cur);
    }
}

/// Переключить видимость resize-handle'ов `TransformBox`'а.
pub fn toggle_resize_mode(node: &NodeInstance) {
    let rt = node.runtime.lock().unwrap_or_else(|e| e.into_inner());
    if let NodeRuntime::MarkdownView { resize_mode, .. } = &*rt {
        let cur = resize_mode.get_untracked();
        resize_mode.set(!cur);
    }
}

/// Body builder, регистрируемый в `NodeKindMeta::body`.
pub fn body(node: &NodeInstance) -> Box<dyn Widget> {
    let (content, edit_mode, resize_mode, size) = {
        let rt = node.runtime.lock().unwrap_or_else(|e| e.into_inner());
        match &*rt {
            NodeRuntime::MarkdownView { content, edit_mode, resize_mode, size } => {
                (*content, *edit_mode, *resize_mode, *size)
            }
            _ => unreachable!("markdown_view::body called for non-MarkdownView kind"),
        }
    };

    // Содержимое body — реактивно переключается между отрендеренным
    // markdown и текстовым редактором по edit_mode. Ширина MarkdownView
    // привязана к size.width, чтобы wrap пересчитывался при resize.
    //
    // Класс `markdown-node` ставится напрямую на сам виджет через
    // WidgetExt::class — `MarkdownView::apply_computed_style` читает
    // `--md-*` переменные только из своего собственного ComputedStyle,
    // а не из ancestor'ов (var()-каскад в syngui MSS не пробрасывает
    // custom properties через дерево).
    let inner = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let sz = size.get();
        if edit_mode.get() {
            vec![Box::new(
                MarkdownEditor::new(content)
                    .initial_mode(EditorMode::Edit)
                    .show_toolbar(false)
                    .syntax_highlight(true)
                    .class("markdown-node"),
            )]
        } else {
            vec![Box::new(
                MarkdownView::new(content.get())
                    .with_syntax_highlight(true)
                    .with_copy_code(true)
                    .max_width((sz.width - 16.0).max(80.0))
                    .selectable(true)
                    .class("markdown-node"),
            )]
        }
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
            .min_size(200.0, 120.0)
            .child(inner),
    )
}
