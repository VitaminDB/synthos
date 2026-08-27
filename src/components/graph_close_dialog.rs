//! Portal-диалог «закрыть граф с несохранёнными изменениями?».
//!
//! Источник — [`EditorWorkspace::pending_close`]: «Закрыть» в контекстном
//! меню плитки графа только взводит сигнал (если граф dirty), а решение —
//! сохранить как шаблон, закрыть без сохранения или передумать — принимается
//! здесь. Смонтирован один раз в shell'е (`lib.rs::build_app`), потому что
//! закрыть плитку можно с любой страницы.
//!
//! Разметка и классы — те же, что у `code_editor::dialogs`
//! (`code-editor-dialog-*`), чтобы все подтверждения выглядели одинаково.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;

use crate::components::template_picker;
use crate::icons::{MI_CLOSE, MI_DELETE, MI_SAVE};
use crate::pages::node_editor::tabs::{EditorWorkspace, OpenTab};

pub fn view() -> impl Widget {
    let is_open = use_signal(false);
    create_effect(move || {
        let ws = use_context::<EditorWorkspace>();
        let has = ws.pending_close.get().is_some();
        if is_open.get_untracked() != has {
            is_open.set(has);
        }
    });

    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .on_close(|| {
            use_context::<EditorWorkspace>().pending_close.set(None);
        })
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ws = use_context::<EditorWorkspace>();
            let Some(tab) = ws.pending_close.get().and_then(|id| ws.tab(id)) else {
                return vec![Box::new(
                    DecoratedBox::new().class("code-editor-dialog-empty"),
                )];
            };
            vec![Box::new(confirm_card(tab))]
        }))
}

fn confirm_card(tab: OpenTab) -> impl Widget {
    let id = tab.id;
    let title = tab.title.get_untracked();
    let discard = move || {
        let ws = use_context::<EditorWorkspace>();
        ws.pending_close.set(None);
        ws.close(id);
    };
    let save = move || {
        let ws = use_context::<EditorWorkspace>();
        if template_picker::save_tab_as_template(&tab) {
            ws.pending_close.set(None);
            ws.close(id);
        }
    };
    let cancel = || {
        use_context::<EditorWorkspace>().pending_close.set(None);
    };

    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card code-editor-dialog-danger") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("nodes.close_dialog.title", title = title))
                        .class("code-editor-dialog-title"),
                    Text::new(tr!("nodes.close_dialog.hint")).class("code-editor-dialog-hint"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("code-editor-dialog-btn-secondary"),
                            Button::new(tr!("nodes.close_dialog.discard"))
                                .leading_icon(MI_DELETE)
                                .on_click(discard)
                                .class("code-editor-dialog-btn-danger"),
                            Button::new(tr!("nodes.close_dialog.save"))
                                .leading_icon(MI_SAVE)
                                .on_click(save)
                                .class("code-editor-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}
