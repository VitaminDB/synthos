//! Модальные диалоги SynExplorer: NewPackage / ConfirmDeleteFile /
//! ConfirmCloseUnsaved / RenameFile / Error.
//!
//! Один Portal в `view()` слушает `ctx.pending_dialog` и подменяет содержимое
//! по варианту. Закрытие — Esc, клик вне модала, кнопка отмены.

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::{Checkbox, ProgressBar, TextField};

use crate::icons::{MI_ADD, MI_CHECK, MI_CLOSE, MI_DELETE, MI_FOLDER_OPEN, MI_SAVE};

use super::actions;
use super::bundle_io;
use super::state::{
    CreateProgress, DialogKind, LoadState, NewPackageComponent, NewPackageForm, SynExplorerCtx,
};

pub fn view() -> impl Widget {
    let is_open = use_signal(false);
    create_effect(move || {
        let ctx = use_context::<SynExplorerCtx>();
        let has = ctx.pending_dialog.get().is_some();
        if is_open.get_untracked() != has {
            is_open.set(has);
        }
    });

    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynExplorerCtx>();
            let Some(kind) = ctx.pending_dialog.get() else {
                return vec![Box::new(DecoratedBox::new().class("syn-dialog-empty"))];
            };
            let card: Box<dyn Widget> = match kind {
                DialogKind::NewPackage => Box::new(new_package_card(ctx.new_package_form)),
                DialogKind::ConfirmDeleteFile { name } => Box::new(confirm_delete_card(name)),
                DialogKind::ConfirmCloseUnsaved => Box::new(confirm_close_card()),
                DialogKind::RenameFile { old } => Box::new(rename_card(old)),
                DialogKind::Error { title, message } => Box::new(error_card(title, message)),
            };
            vec![card]
        }))
}

fn confirm_delete_card(name: String) -> impl Widget {
    let name_for_btn = name.clone();
    let confirm = move || {
        let ctx = use_context::<SynExplorerCtx>();
        if let Some(active) = ctx.active_untracked() {
            actions::confirm_delete_file(active, name_for_btn.clone());
        }
        ctx.close_dialog();
    };
    let cancel = || {
        let ctx = use_context::<SynExplorerCtx>();
        ctx.close_dialog();
    };
    mgui! {
        DecoratedBox::new().class("syn-dialog-card syn-dialog-danger") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("explorer.dialog.confirm_delete.title")).class("syn-dialog-title"),
                    Text::new(tr!("explorer.dialog.confirm_delete.hint", name = name))
                        .class("syn-dialog-hint"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("syn-dialog-btn-secondary"),
                            Button::new(tr!("app.delete"))
                                .leading_icon(MI_DELETE)
                                .on_click(confirm)
                                .class("syn-dialog-btn-danger"),
                        ],
                ]
        ]
    }
}

fn confirm_close_card() -> impl Widget {
    let proceed = || {
        let ctx = use_context::<SynExplorerCtx>();
        actions::force_close_active_bundle(ctx);
        ctx.close_dialog();
    };
    let reload = || {
        let ctx = use_context::<SynExplorerCtx>();
        actions::force_reload_active(ctx);
        ctx.close_dialog();
    };
    let cancel = || {
        let ctx = use_context::<SynExplorerCtx>();
        ctx.close_dialog();
    };
    mgui! {
        DecoratedBox::new().class("syn-dialog-card") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("explorer.unsaved_edits")).class("syn-dialog-title"),
                    Text::new(tr!("explorer.dialog.confirm_close.hint"))
                        .class("syn-dialog-hint"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("syn-dialog-btn-secondary"),
                            Button::new(tr!("explorer.dialog.confirm_close.reload"))
                                .on_click(reload)
                                .class("syn-dialog-btn-secondary"),
                            Button::new(tr!("explorer.dialog.confirm_close.discard"))
                                .leading_icon(MI_DELETE)
                                .on_click(proceed)
                                .class("syn-dialog-btn-danger"),
                        ],
                ]
        ]
    }
}

fn rename_card(old: String) -> impl Widget {
    // RenameFile в SynExplorer не вызывается из toolbar — оставляем
    // диалог как заглушку для будущего; UI и handler есть.
    let ctx = use_context::<SynExplorerCtx>();
    ctx.rename_buffer.set(old.clone());
    let old_for_action = old.clone();
    let buffer = ctx.rename_buffer;

    let confirm = move || {
        let new_name = buffer.get_untracked();
        let ctx = use_context::<SynExplorerCtx>();
        if let Some(active) = ctx.active_untracked() {
            if !new_name.is_empty() && new_name != old_for_action {
                active.pending_ops.update(|v| {
                    v.push(super::state::PendingOp::Rename {
                        old: old_for_action.clone(),
                        new: new_name.clone(),
                    });
                });
            }
        }
        ctx.close_dialog();
    };
    let cancel = || {
        let ctx = use_context::<SynExplorerCtx>();
        ctx.close_dialog();
    };

    mgui! {
        DecoratedBox::new().class("syn-dialog-card") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("explorer.dialog.rename.title")).class("syn-dialog-title"),
                    Text::new(tr!("explorer.dialog.rename.current_name", name = old)).class("syn-dialog-hint"),
                    TextField::with_text(old.clone())
                        .placeholder(tr!("explorer.dialog.rename.placeholder"))
                        .on_change(move |s| buffer.set(s.to_string()))
                        .class("syn-dialog-input"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("syn-dialog-btn-secondary"),
                            Button::new(tr!("app.ok"))
                                .leading_icon(MI_CHECK)
                                .on_click(confirm)
                                .class("syn-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}

fn error_card(title: String, message: String) -> impl Widget {
    let close = || {
        let ctx = use_context::<SynExplorerCtx>();
        ctx.close_dialog();
    };
    mgui! {
        DecoratedBox::new().class("syn-dialog-card syn-dialog-error") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(title).class("syn-dialog-title"),
                    Text::new(message).class("syn-dialog-error-body"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.ok"))
                                .leading_icon(MI_CHECK)
                                .on_click(close)
                                .class("syn-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}

fn new_package_card(form: NewPackageForm) -> impl Widget {
    let id_field = TextField::with_text(form.id.get_untracked())
        .placeholder(tr!("explorer.dialog.new_package.id_placeholder"))
        .on_change(move |s| form.id.set(s.to_string()))
        .class("syn-dialog-input");
    let version_field = TextField::with_text(form.version.get_untracked())
        .placeholder("1.0.0")
        .on_change(move |s| form.version.set(s.to_string()))
        .class("syn-dialog-input");
    let arch_field = TextField::with_text(form.arch.get_untracked())
        .placeholder("xlm-roberta / llama / …")
        .on_change(move |s| form.arch.set(s.to_string()))
        .class("syn-dialog-input");
    let purpose_field = TextField::with_text(form.purpose.get_untracked())
        .placeholder("embed / asr / tts / music …")
        .on_change(move |s| form.purpose.set(s.to_string()))
        .class("syn-dialog-input");

    let pick_out = move || actions::pick_out_path_for_new(form);

    let out_label = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let v = form
            .out_path
            .get()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| tr!("explorer.dialog.new_package.not_selected"));
        vec![Box::new(Text::new(tr!("explorer.dialog.new_package.out_file", path = v)).class("syn-dialog-path"))]
    });

    // Список компонент — реактивный: при добавлении/удалении пересобираем UI.
    // Оборачиваем в один Column — Reactive раскладывает детей через
    // LayoutHint::Loose (stack-style), без Column они бы наложились друг
    // на друга.
    let components_view = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let comps = form.components.get();
        let total = comps.len();
        let mut rows: Vec<Box<dyn Widget>> = Vec::new();
        for (i, c) in comps.into_iter().enumerate() {
            rows.push(Box::new(component_row(form, c, i, total)));
        }
        rows.push(Box::new(
            Button::new(tr!("explorer.dialog.new_package.add_component"))
                .leading_icon(MI_ADD)
                .on_click(move || actions::add_component(form))
                .class("syn-dialog-btn-secondary"),
        ));
        let column = Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows);
        vec![Box::new(column)]
    });

    // Чекбокс «удалить исходники» — деструктивная опция, явно подсвечиваем
    // классом `.syn-dialog-warning` в MSS.
    let delete_sources_sig = form.delete_sources;
    let delete_checkbox = Checkbox::checked(delete_sources_sig.get_untracked())
        .label(tr!("explorer.dialog.new_package.delete_sources"))
        .on_change(move |v| delete_sources_sig.set(v))
        .class("syn-dialog-checkbox");

    // Прогресс-зона: пуста в LoadState::Idle, активна в LoadState::Creating.
    // Подписываемся на `create_progress_gen` чтобы пересоберать UI на каждый
    // throttled tick из worker'а.
    let progress_zone = Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<SynExplorerCtx>();
        let state = ctx.load_state.get();
        let _gen = ctx.create_progress_gen.get();
        if !matches!(state, LoadState::Creating) {
            return vec![];
        }
        let snapshot = read_progress_snapshot(&ctx);
        vec![Box::new(progress_panel(snapshot))]
    });

    let confirm = move || {
        let ctx = use_context::<SynExplorerCtx>();
        // НЕ закрываем диалог — пользователь увидит прогресс прямо в нём.
        actions::create_bundle(ctx);
    };
    let cancel = || {
        let ctx = use_context::<SynExplorerCtx>();
        // Запрет отмены во время Creating — write_async всё равно работает,
        // а полу-закрытый диалог сбил бы реактивность прогресс-панели.
        if matches!(ctx.load_state.get_untracked(), LoadState::Creating) {
            return;
        }
        ctx.close_dialog();
    };

    let _ = bundle_io::get_bundle; // suppress unused warning if any
    mgui! {
        DecoratedBox::new().class("syn-dialog-card syn-dialog-wide") => [
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("explorer.dialog.new_package.title")).class("syn-dialog-title"),
                    Text::new(tr!("explorer.dialog.new_package.hint"))
                        .class("syn-dialog-hint"),
                    field_row("id", id_field),
                    field_row("version", version_field),
                    field_row("arch", arch_field),
                    field_row("purpose", purpose_field),
                    Text::new(tr!("explorer.dialog.new_package.components_label"))
                        .class("syn-dialog-section-label"),
                    components_view,
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center) => [
                            Button::new(tr!("explorer.dialog.new_package.pick_out"))
                                .leading_icon(MI_SAVE)
                                .on_click(pick_out)
                                .class("syn-dialog-btn-secondary"),
                            out_label,
                        ],
                    delete_checkbox,
                    progress_zone,
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("syn-dialog-btn-secondary"),
                            Button::new(tr!("explorer.dialog.new_package.create"))
                                .leading_icon(MI_CHECK)
                                .on_click(confirm)
                                .class("syn-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}

/// Снять консистентный слепок прогресса для одного UI-тика. Захватываем
/// Mutex кратко — без удержания через render.
fn read_progress_snapshot(ctx: &SynExplorerCtx) -> CreateProgress {
    let handle = ctx.create_progress.get_untracked();
    handle.lock().ok().map(|g| g.clone()).unwrap_or_default()
}

fn progress_panel(p: CreateProgress) -> impl Widget {
    let fraction = p.fraction();
    let percent = (fraction * 100.0).round() as i32;
    // Отображаем реальный размер payload'а (без удвоения stage+pack) —
    // пользователь видит «X из 51 ГБ», а не «X из 103 ГБ».
    let bytes_human = tr!(
        "explorer.dialog.new_package.progress_bytes",
        done = format!("{:.2}", p.display_done() as f64 / (1024.0 * 1024.0 * 1024.0)),
        total = format!("{:.2}", p.payload_total as f64 / (1024.0 * 1024.0 * 1024.0))
    );
    let stage = if p.stage_label.is_empty() {
        tr!("explorer.dialog.new_package.stage_preparing")
    } else {
        p.stage_label
    };
    let is_indet = p.finalizing || p.bytes_total == 0;
    // `Reactive` оборачивает выбор бар-варианта в виджет, понятный mgui!-макросу:
    // mgui! не принимает Box<dyn Widget> напрямую (требует IntoWidget).
    let bar = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if is_indet {
            vec![Box::new(
                ProgressBar::new()
                    .indeterminate()
                    .class("syn-dialog-progress"),
            )]
        } else {
            vec![Box::new(
                ProgressBar::with_value(fraction).class("syn-dialog-progress"),
            )]
        }
    });
    mgui! {
        DecoratedBox::new().class("syn-dialog-progress-card") => [
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(stage).class("syn-dialog-progress-stage"),
                    bar,
                    Row::new()
                        .gap(8.0)
                        .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                            Text::new(bytes_human).class("syn-dialog-progress-bytes"),
                            Text::new(format!("{percent}%"))
                                .class("syn-dialog-progress-percent"),
                        ],
                ]
        ]
    }
}

fn component_row(
    form: NewPackageForm,
    c: NewPackageComponent,
    index: usize,
    total: usize,
) -> impl Widget {
    let name_field = TextField::with_text(c.name.get_untracked())
        .placeholder("main / lm / codec …")
        .on_change(move |s| c.name.set(s.to_string()))
        .class("syn-dialog-input");
    let prefix_field = TextField::with_text(c.prefix.get_untracked())
        .placeholder(tr!("explorer.dialog.new_package.component.prefix_placeholder"))
        .on_change(move |s| c.prefix.set(s.to_string()))
        .class("syn-dialog-input");

    let is_first = index == 0;
    let pick = move || actions::pick_source_dir_for_component(form, c, is_first);
    let remove = move || actions::remove_component(form, index);

    let dir_label = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let v = c
            .source_dir
            .get()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| tr!("explorer.dialog.new_package.component.dir_not_selected"));
        vec![Box::new(Text::new(v).class("syn-dialog-path"))]
    });

    // `total` Copy → захватываем в Reactive: пока компонентов больше одного,
    // показываем delete-кнопку, иначе пусто (последний компонент не удалить).
    let remove_btn = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if total > 1 {
            vec![Box::new(
                Button::new("")
                    .leading_icon(MI_DELETE)
                    .on_click(move || actions::remove_component(form, index))
                    .class("syn-dialog-btn-icon"),
            )]
        } else {
            vec![]
        }
    });
    let _ = remove;

    mgui! {
        DecoratedBox::new().class("syn-dialog-component-card") => [
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center) => [
                            Text::new(format!("#{}", index + 1))
                                .class("syn-dialog-component-index"),
                            name_field,
                            prefix_field,
                            remove_btn,
                        ],
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center) => [
                            Button::new(tr!("explorer.dialog.new_package.component.pick_folder"))
                                .leading_icon(MI_FOLDER_OPEN)
                                .on_click(pick)
                                .class("syn-dialog-btn-secondary"),
                            dir_label,
                        ],
                ]
        ]
    }
}

fn field_row<W: Widget + 'static>(label: &str, field: W) -> impl Widget {
    let label = label.to_string();
    mgui! {
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(label).class("syn-dialog-label"),
                field,
            ]
    }
}
