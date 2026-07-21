//! Центральная верхняя панель — редактор активного файла активной сессии.
//!
//! Структура:
//! - header: имя активного файла + dirty-индикатор + кнопка Save
//! - body: Reactive(CodeEditor) подписан на `editor_gen` (для пересоздания
//!   виджета при смене файла) и `active_file` (для обновления header'а)
//!
//! Ключевое правило: внутри Reactive-замыкания текст и dirty-флаг читаются
//! через `get_untracked`. Подписка на `file_contents` или `disk_contents`
//! привела бы к циклу: `on_change → update → rebuild → новый CodeEditor
//! → курсор сбрасывается на каждом нажатии клавиши.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::input::code_editor::{languages, EditorCommand, EditorPersistedState};
use syngui::widgets::overlay::context_menu::ContextMenu;
use syngui::widgets::overlay::menu::MenuItem;
use syngui::widgets::CodeEditor;

use crate::icons::{
    MI_CONTENT_COPY, MI_CONTENT_CUT, MI_CONTENT_PASTE, MI_DESCRIPTION, MI_HISTORY, MI_SAVE,
    MI_SELECT_ALL, MI_WRAP_TEXT,
};

use super::dialogs::DialogKind;
use super::file_icons;
use super::state::{self, is_dirty, CodeEditorCtx, CodeSession};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("code-editor-edit-pane").child(mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                header(),
                body(),
            ]
    })
}

fn header() -> impl Widget {
    DecoratedBox::new().class("code-editor-edit-header").child(move || {
        let code = use_context::<CodeEditorCtx>();
        let child: Box<dyn Widget> = match code.active_session() {
            None => Box::new(DecoratedBox::new()),
            Some(session) => {
                let active = session.active_file.get();
                let contents = session.file_contents.get();
                let disk = session.disk_contents.get();

                let label = active
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| String::from("Файл не выбран"));
                let dirty = active
                    .as_ref()
                    .map(|p| is_dirty(&contents, &disk, p))
                    .unwrap_or(false);

                let dirty_class = if dirty {
                    "code-editor-dirty-dot active"
                } else {
                    "code-editor-dirty-dot"
                };
                let save_class = if dirty {
                    "code-editor-save-btn enabled"
                } else {
                    "code-editor-save-btn"
                };
                let wrap_enabled = session.soft_wrap.get();
                let wrap_class = if wrap_enabled {
                    "code-editor-wrap-btn enabled"
                } else {
                    "code-editor-wrap-btn"
                };

                // Иконка типа активного файла. Для пустого active —
                // generic MI_DESCRIPTION (placeholder будет «Файл не выбран»).
                let (header_icon, header_icon_class) = match active.as_ref() {
                    Some(p) => (
                        file_icons::icon_for_path(p, false, false),
                        format!(
                            "code-editor-edit-icon {}",
                            file_icons::class_for_path(p, false, false)
                        ),
                    ),
                    None => (MI_DESCRIPTION, "code-editor-edit-icon".to_string()),
                };

                Box::new(mgui! {
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center) => [
                            Icon::new(header_icon).class(header_icon_class),
                            Text::new(label).class("code-editor-edit-filename"),
                            DecoratedBox::new().class(dirty_class),
                            DecoratedBox::new().class("grow"),
                            ToolButton::new(MI_WRAP_TEXT)
                                .tooltip("Перенос строк (word wrap)")
                                .on_click(move || {
                                    session.soft_wrap.update(|b| *b = !*b);
                                    // Toggle также перерисовывает CodeEditor
                                    // через `editor_gen` — Reactive в `body()`
                                    // подписан на `editor_gen` и пересоздаст
                                    // виджет с новым `.soft_wrap(...)`.
                                    session.editor_gen.update(|n| *n = n.wrapping_add(1));
                                })
                                .class(wrap_class),
                            ToolButton::new(MI_HISTORY)
                                .tooltip("История версий")
                                .on_click({
                                    let active = active.clone();
                                    move || {
                                        if let Some(p) = active.clone() {
                                            let code = use_context::<CodeEditorCtx>();
                                            code.open_dialog(DialogKind::History { path: p });
                                        }
                                    }
                                })
                                .class("code-editor-history-btn"),
                            ToolButton::new(MI_SAVE)
                                .tooltip("Сохранить (на диск)")
                                .on_click(move || {
                                    let code = use_context::<CodeEditorCtx>();
                                    if let Some(s) = code.active_session_untracked() {
                                        state::save_active(s);
                                    }
                                })
                                .class(save_class),
                        ]
                })
            }
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    })
}

fn body() -> impl Widget {
    DecoratedBox::new().class("code-editor-edit-body grow").child(move || {
        let code = use_context::<CodeEditorCtx>();

        let child: Box<dyn Widget> = match code.active_session() {
            None => Box::new(empty_state()),
            Some(session) => {
                // Подписки только на «структурные» сигналы — на content'ы НЕ подписываемся,
                // иначе курсор будет сбрасываться при каждом keystroke.
                let _ = session.editor_gen.get();
                let active = session.active_file.get();
                match active {
                    Some(path) => {
                        let initial = session
                            .file_contents
                            .get_untracked()
                            .get(&path)
                            .cloned()
                            .unwrap_or_default();
                        editor_widget(session, initial)
                    }
                    None => Box::new(empty_state()),
                }
            }
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    })
}

fn editor_widget(session: CodeSession, initial: String) -> Box<dyn Widget> {
    let active = session.active_file.get_untracked();
    let language = active
        .as_ref()
        .and_then(|p| languages::detect_by_path(p));
    // Канал команд от ContextMenu (Copy/Cut/Paste/Select All) → CodeEditor.
    // Сигнал создаётся локально на каждый rebuild editor'а (это нормально:
    // меню и редактор всегда живут парой и пересоздаются вместе).
    let cmd: RwSignal<Option<EditorCommand>> = use_signal(None);
    let soft_wrap = session.soft_wrap.get_untracked();

    // Двусторонний канал persisted-состояния (cursor + scroll). Инициализируем
    // из `session.editor_states[active_file]` (если есть) — редактор на mount
    // прочитает `get_untracked()` и восстановит позицию. Каждое изменение
    // (cursor move / scroll) редактор пишет обратно; `create_effect` ниже
    // забирает значение и сохраняет в `session.editor_states` под ключом
    // активного файла, что подхватит autosave конфига.
    let initial_state = active
        .as_ref()
        .and_then(|p| session.editor_states.get_untracked().get(p).copied())
        .unwrap_or_default();
    let state_signal: RwSignal<EditorPersistedState> = use_signal(initial_state);
    let active_path_untracked = active.clone();
    create_effect(move || {
        let snap = state_signal.get();
        if let Some(path) = active_path_untracked.clone() {
            session.editor_states.update(|m| {
                m.insert(path, snap);
            });
        }
    });

    // External-reload канал: при изменении `file_contents[active]` (fs_watcher
    // подхватил внешнюю правку — `git pull`, другой редактор, agent через
    // Edit-tool) пушим Reload(text) в command_signal — CodeEditor заменит
    // содержимое buffer без пересоздания виджета (курсор/скролл сохраняются).
    // Защита от self-loop: запоминаем последний "виденный" текст в локальной
    // RwSignal<String>, и не дергаем Reload пока он не изменится. Это критично,
    // т.к. on_change тоже пишет в file_contents — без guard'а получим цикл
    // keystroke → file_contents → Reload → buffer reset на каждое нажатие.
    let active_for_reload = active.clone();
    let last_seen_text: RwSignal<String> = use_signal(active_for_reload
        .as_ref()
        .and_then(|p| session.file_contents.get_untracked().get(p).cloned())
        .unwrap_or_default());
    create_effect(move || {
        let map = session.file_contents.get();
        if let Some(path) = active_for_reload.clone() {
            if let Some(v) = map.get(&path) {
                let prev = last_seen_text.get_untracked();
                if prev != *v {
                    last_seen_text.set(v.clone());
                    cmd.set(Some(EditorCommand::Reload(v.clone())));
                }
            }
        }
    });

    let mut editor = CodeEditor::new()
        .text(initial)
        .show_line_numbers(true)
        .tab_width(4)
        .insert_spaces(true)
        .soft_wrap(soft_wrap)
        .on_change(move |change| state::update_active_text(session, change.full_text))
        .on_save(move |_text| state::save_active(session))
        .command_signal(cmd)
        .state_signal(state_signal)
        .class("code-editor-mle");
    if let Some(lang) = language {
        editor = editor.language(lang);
    }
    let menu = ContextMenu::new()
        .child(editor)
        .items(vec![
            MenuItem::new("copy", "Копировать")
                .icon(MI_CONTENT_COPY)
                .shortcut("Ctrl+C"),
            MenuItem::new("cut", "Вырезать")
                .icon(MI_CONTENT_CUT)
                .shortcut("Ctrl+X"),
            MenuItem::new("paste", "Вставить")
                .icon(MI_CONTENT_PASTE)
                .shortcut("Ctrl+V"),
            MenuItem::separator(),
            MenuItem::new("select_all", "Выделить всё")
                .icon(MI_SELECT_ALL)
                .shortcut("Ctrl+A"),
        ])
        .on_select(move |id| {
            let action = match id {
                "copy" => Some(EditorCommand::Copy),
                "cut" => Some(EditorCommand::Cut),
                "paste" => Some(EditorCommand::Paste),
                "select_all" => Some(EditorCommand::SelectAll),
                _ => None,
            };
            if let Some(a) = action {
                cmd.set(Some(a));
            }
        });
    Box::new(menu)
}

fn empty_state() -> impl Widget {
    Center::new().child(mgui! {
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_DESCRIPTION).class("code-editor-edit-empty-icon"),
                Text::new("Откройте файл из дерева слева").class("code-editor-edit-empty-title"),
                Text::new("Файл появится здесь и в правом сайдбаре").class("code-editor-edit-empty-hint"),
            ]
    })
}
