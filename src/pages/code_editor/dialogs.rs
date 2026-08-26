//! Модальные диалоги редактора кода: New file / New folder / Rename / Delete.
//!
//! Все диалоги живут в одном [`Portal`] (overlay-слой), смонтированном
//! один раз в `code_editor::view()`. Видимость и тип диалога управляются
//! сигналом [`CodeEditorCtx::pending_dialog`] — UI читает его реактивно
//! и подменяет содержимое Portal'а в зависимости от kind'а.
//!
//! Принцип: один Portal-инстанс, разные children по типу. Это упрощает
//! монтирование и закрытие — не нужно поддерживать отдельные Portal'ы
//! для каждого диалога. Диалог закрывается через
//! [`CodeEditorCtx::close_dialog`] либо пользователем (Esc / клик вне /
//! кнопка отмены).

use std::path::PathBuf;

use syngui::context_provider::use_context;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::TextField;

use crate::icons::{
    MI_CHECK, MI_CLOSE, MI_DELETE, MI_DOWNLOAD, MI_HISTORY, MI_MERGE_TYPE, MI_SYNC_PROBLEM,
};

use super::drafts;
use super::fs_actions;
use super::state::{CodeEditorCtx, CodeSession};
use super::text_diff::{self, DiffKind};

/// Тип открытого диалога. Хранится в `CodeEditorCtx::pending_dialog`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogKind {
    /// Новый файл внутри родительской папки. Поле — пустая строка по умолчанию.
    NewFile { parent: PathBuf },
    /// Новая папка внутри родительской папки.
    NewFolder { parent: PathBuf },
    /// Переименовать существующий путь. Pre-fill поля — текущее имя
    /// (basename), курсор стоит перед расширением.
    Rename { path: PathBuf },
    /// Подтверждение удаления. Без TextField, только confirm-кнопка.
    Delete { path: PathBuf },
    /// Внешнее изменение конфликтует с dirty-буфером: выбор разрешения.
    ExternalConflict { path: PathBuf },
    /// История версий файла (Local History): список снимков + восстановление.
    History { path: PathBuf },
}

/// Корневой view диалогов. Монтируется в `code_editor::view()` поверх Stack'а.
/// Portal сам ничего не рисует, пока `pending_dialog = None` — `is_open`
/// зеркалится в отдельный сигнал через `create_effect` (см. реализацию).
///
/// Содержимое card'а реактивно: разные `DialogKind` рендерятся разными
/// картами. Reactive нужен явно, потому что match-ветки возвращают разные
/// конкретные типы — IntoWidget через `Fn() -> impl Widget` требовал бы
/// одного типа возврата.
pub fn view() -> impl Widget {
    let is_open = use_signal(false);
    create_effect(move || {
        let code = use_context::<CodeEditorCtx>();
        let has = code.pending_dialog.get().is_some();
        if is_open.get_untracked() != has {
            is_open.set(has);
        }
    });

    create_effect(move || {
        let code = use_context::<CodeEditorCtx>();
        let _ = code.pending_dialog.get();
        let Some(session) = code.active_session() else {
            return;
        };
        let conflicts = session.conflicts.get();
        if conflicts.is_empty() || code.pending_dialog.get_untracked().is_some() {
            return;
        }
        if let Some(first) = conflicts.first() {
            code.open_dialog(DialogKind::ExternalConflict { path: first.clone() });
        }
    });

    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .on_close(|| {
            use_context::<CodeEditorCtx>().close_dialog();
        })
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let code = use_context::<CodeEditorCtx>();
            let Some(kind) = code.pending_dialog.get() else {
                return vec![Box::new(
                    DecoratedBox::new().class("code-editor-dialog-empty"),
                )];
            };
            let Some(session) = code.active_session_untracked() else {
                code.close_dialog();
                return vec![Box::new(
                    DecoratedBox::new().class("code-editor-dialog-empty"),
                )];
            };

            let card: Box<dyn Widget> = match kind {
                DialogKind::NewFile { parent } => Box::new(text_input_card(
                    tr!("code.dialog.new_file.title"),
                    tr!("code.dialog.create_in", dir = short_dir(&parent)),
                    tr!("code.dialog.new_file.placeholder"),
                    String::new(),
                    move |name| fs_actions::create_file(session, parent.clone(), name),
                )),
                DialogKind::NewFolder { parent } => Box::new(text_input_card(
                    tr!("code.dialog.new_folder.title"),
                    tr!("code.dialog.create_in", dir = short_dir(&parent)),
                    tr!("code.dialog.new_folder.placeholder"),
                    String::new(),
                    move |name| fs_actions::create_folder(session, parent.clone(), name),
                )),
                DialogKind::Rename { path } => {
                    let initial = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let path_for_action = path.clone();
                    Box::new(text_input_card(
                        tr!("code.dialog.rename.title"),
                        tr!("code.dialog.rename.current", path = path.display().to_string()),
                        tr!("code.dialog.rename.placeholder"),
                        initial,
                        move |name| fs_actions::rename(session, path_for_action.clone(), name),
                    ))
                }
                DialogKind::Delete { path } => Box::new(delete_confirm_card(session, path)),
                DialogKind::ExternalConflict { path } => {
                    Box::new(external_conflict_card(session, path))
                }
                DialogKind::History { path } => Box::new(history_card(session, path)),
            };
            vec![card]
        }))
}

/// Карточка с полем ввода и кнопками [Отмена] [Создать/Переименовать].
/// Reactive внутри title'а / hint'а не нужен — все три параметра
/// фиксированы на момент открытия диалога.
fn text_input_card<F>(
    title: impl Into<String>,
    hint: impl Into<String>,
    placeholder: impl Into<String>,
    initial: String,
    on_confirm: F,
) -> impl Widget
where
    F: Fn(String) + Send + Sync + Clone + 'static,
{
    // Локальный сигнал значения. Перехватывается каждым keystroke в TextField,
    // на confirm читается через get_untracked.
    let value = use_signal(initial.clone());
    let title = title.into();
    let hint = hint.into();
    let placeholder = placeholder.into();

    let on_confirm_btn = on_confirm.clone();
    let confirm_action = move || {
        let v = value.get_untracked();
        on_confirm_btn(v);
        let code = use_context::<CodeEditorCtx>();
        code.close_dialog();
    };
    let cancel_action = || {
        let code = use_context::<CodeEditorCtx>();
        code.close_dialog();
    };

    let confirm_for_field = on_confirm.clone();
    let value_for_field = value;

    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(title).class("code-editor-dialog-title"),
                    Text::new(hint).class("code-editor-dialog-hint"),
                    TextField::new()
                        .text(initial)
                        .placeholder(placeholder)
                        .on_change(move |s| value_for_field.set(s.to_string()))
                        .on_submit(move |s| {
                            confirm_for_field(s.to_string());
                            let code = use_context::<CodeEditorCtx>();
                            code.close_dialog();
                        })
                        .class("code-editor-dialog-input"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel_action)
                                .class("code-editor-dialog-btn-secondary"),
                            Button::new(tr!("app.ok"))
                                .leading_icon(MI_CHECK)
                                .on_click(confirm_action)
                                .class("code-editor-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}

fn delete_confirm_card(session: CodeSession, path: PathBuf) -> impl Widget {
    let path_for_btn = path.clone();
    let confirm = move || {
        fs_actions::delete(session, path_for_btn.clone());
        let code = use_context::<CodeEditorCtx>();
        code.close_dialog();
    };
    let cancel = || {
        let code = use_context::<CodeEditorCtx>();
        code.close_dialog();
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let kind_label = if path.is_dir() {
        tr!("code.dialog.delete.kind_folder")
    } else {
        tr!("code.dialog.delete.kind_file")
    };

    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card code-editor-dialog-danger") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("code.dialog.delete.title", kind = kind_label)).class("code-editor-dialog-title"),
                    Text::new(tr!("code.dialog.delete.hint", name = name))
                        .class("code-editor-dialog-hint"),
                    Text::new(path.display().to_string())
                        .class("code-editor-dialog-path"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("code-editor-dialog-btn-secondary"),
                            Button::new(tr!("app.delete"))
                                .leading_icon(MI_DELETE)
                                .on_click(confirm)
                                .class("code-editor-dialog-btn-danger"),
                        ],
                ]
        ]
    }
}

fn external_conflict_card(session: CodeSession, path: PathBuf) -> impl Widget {
    let show_diff = use_signal(false);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());

    let keep_path = path.clone();
    let keep = move || {
        session.conflicts.update(|v| v.retain(|p| p != &keep_path));
        use_context::<CodeEditorCtx>().close_dialog();
    };

    let reload_path = path.clone();
    let reload = move || {
        let mine = session
            .file_contents
            .get_untracked()
            .get(&reload_path)
            .cloned()
            .unwrap_or_default();
        drafts::snapshot_history(&reload_path, &mine);
        let disk = session
            .disk_contents
            .get_untracked()
            .get(&reload_path)
            .cloned()
            .unwrap_or_default();
        session.file_contents.update(|m| {
            m.insert(reload_path.clone(), disk);
        });
        session.conflicts.update(|v| v.retain(|p| p != &reload_path));
        drafts::clear_draft(&reload_path);
        use_context::<CodeEditorCtx>().close_dialog();
    };

    let toggle_diff = move || {
        show_diff.update(|b| *b = !*b);
    };

    let diff_path = path.clone();
    let diff_area = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        if !show_diff.get() {
            return Vec::new();
        }
        let mine = session
            .file_contents
            .get_untracked()
            .get(&diff_path)
            .cloned()
            .unwrap_or_default();
        let disk = session
            .disk_contents
            .get_untracked()
            .get(&diff_path)
            .cloned()
            .unwrap_or_default();
        vec![Box::new(diff_view(&disk, &mine))]
    });

    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card code-editor-conflict-card") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center) => [
                            Icon::new(MI_SYNC_PROBLEM).class("code-editor-conflict-icon"),
                            Text::new(tr!("code.dialog.conflict.title")).class("code-editor-dialog-title"),
                        ],
                    Text::new(tr!("code.dialog.conflict.hint", name = name))
                    .class("code-editor-dialog-hint"),
                    Text::new(path.display().to_string()).class("code-editor-dialog-path"),
                    diff_area,
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("code.dialog.conflict.show_diff"))
                                .leading_icon(MI_MERGE_TYPE)
                                .on_click(toggle_diff)
                                .class("code-editor-dialog-btn-secondary"),
                            Button::new(tr!("code.dialog.conflict.reload"))
                                .leading_icon(MI_DOWNLOAD)
                                .on_click(reload)
                                .class("code-editor-dialog-btn-secondary"),
                            Button::new(tr!("code.dialog.conflict.keep_mine"))
                                .leading_icon(MI_CHECK)
                                .on_click(keep)
                                .class("code-editor-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}

fn diff_view(old: &str, new: &str) -> impl Widget {
    let lines = text_diff::unified_lines(old, new);
    let mut rows: Vec<Box<dyn Widget>> = Vec::with_capacity(lines.len());
    for line in lines {
        let (cls, prefix) = match line.kind {
            DiffKind::Context => ("code-editor-diff-line", ' '),
            DiffKind::Removed => ("code-editor-diff-line removed", '-'),
            DiffKind::Added => ("code-editor-diff-line added", '+'),
        };
        rows.push(Box::new(
            Text::new(format!("{prefix} {}", line.text)).class(cls),
        ));
    }
    ScrollView::new().vertical().class("code-editor-diff-scroll").child(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}

fn history_card(session: CodeSession, path: PathBuf) -> impl Widget {
    let entries = drafts::list_history(&path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());

    let mut rows: Vec<Box<dyn Widget>> = Vec::new();
    if entries.is_empty() {
        rows.push(Box::new(
            Text::new(tr!("code.dialog.history.empty")).class("code-editor-history-empty"),
        ));
    } else {
        for entry in entries {
            let age = drafts::human_age(entry.saved_at);
            let restore = move || {
                if let Some(text) = drafts::read_history(&entry) {
                    session.file_contents.update(|m| {
                        m.insert(entry.path.clone(), text);
                    });
                    use_context::<CodeEditorCtx>().close_dialog();
                }
            };
            rows.push(Box::new(mgui! {
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("code-editor-history-row") => [
                        Icon::new(MI_HISTORY).class("code-editor-history-row-icon"),
                        Text::new(age).class("code-editor-history-row-age"),
                        DecoratedBox::new().class("grow"),
                        Button::new(tr!("code.dialog.history.restore"))
                            .leading_icon(MI_DOWNLOAD)
                            .on_click(restore)
                            .class("code-editor-dialog-btn-secondary"),
                    ]
            }));
        }
    }

    let close = || {
        let code = use_context::<CodeEditorCtx>();
        code.close_dialog();
    };

    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card code-editor-history-card") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center) => [
                            Icon::new(MI_HISTORY).class("code-editor-history-icon"),
                            Text::new(tr!("code.history.title")).class("code-editor-dialog-title"),
                        ],
                    Text::new(tr!("code.dialog.history.hint", name = name))
                        .class("code-editor-dialog-hint"),
                    ScrollView::new()
                        .vertical()
                        .class("code-editor-history-scroll")
                        .child(
                            Column::new()
                                .gap(6.0)
                                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                                .children(rows),
                        ),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.close"))
                                .leading_icon(MI_CLOSE)
                                .on_click(close)
                                .class("code-editor-dialog-btn-secondary"),
                        ],
                ]
        ]
    }
}

fn short_dir(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.display().to_string())
}
