//! Левая панель — дерево файлов проекта.
//!
//! Структура:
//! - header: имя текущей папки + кнопка «Открыть папку» (rfd::pick_folder)
//! - body: Reactive(TreeView) подписан на `tree_nodes` активной сессии.
//!
//! TreeView UX-нюанс: `on_select` срабатывает для всех узлов (leaf+branch),
//! `on_toggle` — только при клике на чеврон. Чтобы клик по label папки
//! тоже разворачивал её (как в VSCode), в `on_select` для папки вызываем
//! тот же `toggle_dir`, что и в `on_toggle`.
//!
//! Multi-session: вся реактивность подвязана на активную сессию через
//! `code.active_session()`. Подписка на `code.session_gen` происходит на
//! уровне родительского `view()` (в `mod.rs`), который перерисовывается
//! целиком при переключении сессий.

use std::path::PathBuf;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::{ContextMenu, MenuItem};
use syngui::widgets::{SelectionMode, TreeNode, TreeView};

use crate::icons::{
    MI_CONTENT_COPY, MI_CREATE_NEW_FOLDER, MI_DELETE, MI_DRIVE_FILE_RENAME_OUTLINE,
    MI_FOLDER_OPEN, MI_LAUNCH, MI_NOTE_ADD,
};

use super::dialogs::DialogKind;
use super::fs_actions;
use super::fs_ops::PLACEHOLDER_PREFIX;
use super::git_status::{self, GitPalette, GHOST_PREFIX};
use super::state::{self, CodeEditorCtx, CodeSession};

pub fn view() -> impl Widget {
    DecoratedBox::new().class("code-editor-tree-panel").child(mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                header(),
                body(),
            ]
    })
}

fn header() -> impl Widget {
    DecoratedBox::new().class("code-editor-tree-header").child(mgui! {
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                // Имя папки слева. Reactive подписан на root_folder активной сессии.
                Reactive::new(move || -> Vec<Box<dyn Widget>> {
                    let code = use_context::<CodeEditorCtx>();
                    let _ = code.session_gen.get();
                    let name = code
                        .active_session()
                        .and_then(|s| {
                            s.root_folder.get().as_ref().and_then(|p| {
                                p.file_name().map(|n| n.to_string_lossy().to_string())
                            })
                        })
                        .unwrap_or_else(|| tr!("code.tree.header.no_project"));
                    vec![Box::new(Text::new(name).class("code-editor-tree-folder-name"))]
                }),
                // Растягивающийся spacer прижимает ToolButton к правому краю.
                DecoratedBox::new().class("code-editor-header-spacer"),
                // ToolButton — на каждом клике открываем нативный picker.
                // Если активной сессии нет (промежуточное состояние закрытия) —
                // ничего не делаем; нормальный путь — sidebar+/file_tree всегда
                // имеют активную сессию.
                ToolButton::new(MI_FOLDER_OPEN)
                    .on_click(move || {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            let code = use_context::<CodeEditorCtx>();
                            if let Some(session) = code.active_session_untracked() {
                                state::set_root_folder(session, path);
                            }
                        }
                    })
                    .class("code-editor-open-folder-btn"),
            ]
    })
}

fn body() -> impl Widget {
    DecoratedBox::new().class("code-editor-tree-body grow").child(move || {
        let code = use_context::<CodeEditorCtx>();
        // Этот путь редок — body рендерится из mod.rs::view() только при
        // наличии активной сессии, но Reactive может на миг пересчитаться
        // без неё в момент close. Возвращаем placeholder.
        let child: Box<dyn Widget> = match code.active_session() {
            None => Box::new(empty_state()),
            Some(session) => {
                let nodes = session.tree_nodes.get();
                // Подписываемся явно: при изменении selected_node Reactive пересоздаст
                // TreeView, новый виджет получит обновлённое `.selected(...)`. Без
                // этой подписки выделение бы переключалось только на следующем
                // изменении дерева (toggle папки и т. п.) — slow / inconsistent UX.
                let selected = session.selected_node.get();
                // Подписка на git-status: при первом расчёте воркером и
                // на каждом fs_watcher tick'е (через request_git_refresh)
                // карта обновляется → Reactive пересоздаёт TreeView с
                // навешенными декорациями.
                let git = session.git_status.get();
                if nodes.is_empty() {
                    Box::new(empty_state())
                } else {
                    let palette = GitPalette::default();
                    let decorated = git_status::apply_to_nodes(nodes, &git, &palette);
                    tree_widget(session, decorated, selected)
                }
            }
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    })
}

fn empty_state() -> impl Widget {
    Center::new().child(mgui! {
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_FOLDER_OPEN).class("code-editor-empty-icon"),
                Text::new(tr!("code.tree.empty.title")).class("code-editor-empty-title"),
                Text::new(tr!("code.tree.empty.hint")).class("code-editor-empty-hint"),
            ]
    })
}

fn tree_widget(
    session: CodeSession,
    nodes: Vec<TreeNode>,
    selected: Option<String>,
) -> Box<dyn Widget> {
    let tree = TreeView::new(nodes)
        .class("code-editor-tree")
        .indent(18.0)
        .item_height(26.0)
        .selection_mode(SelectionMode::Single)
        // selected живёт снаружи виджета (см. CodeSession::selected_node):
        // Reactive пересоздаёт TreeView при каждом обновлении tree_nodes,
        // и внутреннее selected потерялось бы. Здесь пробрасываем явно.
        .selected(selected.into_iter().collect())
        .on_select(move |id| handle_select(session, id))
        .on_toggle(move |id, _expanded| handle_toggle(session, id));

    // Right-click меню. TreeView сам авто-выделяет узел под курсором на ПКМ
    // и пробрасывает событие наверх (EventResult::Ignored), благодаря чему
    // ContextMenu открывает popup с применением действий к `selected_node`.
    let menu = ContextMenu::new()
        .child(tree)
        .items(menu_items())
        .on_select(move |id| handle_menu(session, id));
    Box::new(menu)
}

fn menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem::new("new_file", tr!("code.dialog.new_file.title")).icon(MI_NOTE_ADD),
        MenuItem::new("new_folder", tr!("code.dialog.new_folder.title")).icon(MI_CREATE_NEW_FOLDER),
        MenuItem::separator(),
        MenuItem::new("rename", tr!("code.dialog.rename.title")).icon(MI_DRIVE_FILE_RENAME_OUTLINE),
        MenuItem::new("delete", tr!("app.delete")).icon(MI_DELETE),
        MenuItem::separator(),
        MenuItem::new("copy_path", tr!("code.tree.menu.copy_path")).icon(MI_CONTENT_COPY),
        MenuItem::new("reveal", tr!("code.tree.menu.reveal")).icon(MI_LAUNCH),
    ]
}

/// Обработать выбор пункта контекстного меню. Целевой путь —
/// `selected_node` (если выбран) или корень проекта (для new file/folder).
fn handle_menu(session: CodeSession, action: &str) {
    let code = syngui::context_provider::use_context::<CodeEditorCtx>();
    let selected = session
        .selected_node
        .get_untracked()
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());
    let root = session.root_folder.get_untracked();

    match action {
        "new_file" | "new_folder" => {
            // Родитель: выбран dir → он сам, выбран file → его parent,
            // ничего не выбрано → корень проекта.
            let parent = match selected.as_ref() {
                Some(p) if p.is_dir() => p.clone(),
                Some(p) => p.parent().map(PathBuf::from).unwrap_or_else(|| {
                    root.clone().unwrap_or_else(|| PathBuf::from("."))
                }),
                None => match root {
                    Some(r) => r,
                    None => {
                        code.show_notice(tr!("code.tree.notice.open_project_first"));
                        return;
                    }
                },
            };
            let kind = if action == "new_file" {
                DialogKind::NewFile { parent }
            } else {
                DialogKind::NewFolder { parent }
            };
            code.open_dialog(kind);
        }
        "rename" => {
            let Some(path) = selected else {
                code.show_notice(tr!("code.tree.notice.select_for_rename"));
                return;
            };
            code.open_dialog(DialogKind::Rename { path });
        }
        "delete" => {
            let Some(path) = selected else {
                code.show_notice(tr!("code.tree.notice.select_for_delete"));
                return;
            };
            code.open_dialog(DialogKind::Delete { path });
        }
        "copy_path" => {
            let Some(path) = selected else {
                code.show_notice(tr!("code.tree.notice.select_for_copy_path"));
                return;
            };
            fs_actions::copy_path_to_clipboard(&path);
        }
        "reveal" => {
            let target = selected.or(root);
            let Some(path) = target else {
                code.show_notice(tr!("code.tree.notice.no_path"));
                return;
            };
            fs_actions::reveal_in_files(&path);
        }
        _ => {}
    }
}

fn handle_select(session: CodeSession, id: &str) {
    if id.starts_with(PLACEHOLDER_PREFIX) {
        return;
    }
    // Ghost-ноды (удалённые файлы): на диске их нет, открыть нельзя.
    // Показываем notice и фиксируем выделение для UX-фидбека.
    if id.starts_with(GHOST_PREFIX) {
        let code = use_context::<CodeEditorCtx>();
        session.selected_node.set(Some(id.to_string()));
        code.show_notice(tr!("code.tree.notice.ghost_file_deleted"));
        return;
    }
    // Сначала фиксируем выделение — для UX-фидбека всегда (и для папок,
    // и для файлов). Потом уже обрабатываем effect клика (open / toggle).
    session.selected_node.set(Some(id.to_string()));

    let path = PathBuf::from(id);
    if path.is_file() {
        state::open_file(session, path);
    } else if path.is_dir() {
        state::toggle_dir(session, path);
    }
}

fn handle_toggle(session: CodeSession, id: &str) {
    if id.starts_with(PLACEHOLDER_PREFIX) {
        return;
    }
    if id.starts_with(GHOST_PREFIX) {
        return;
    }
    let path = PathBuf::from(id);
    if path.is_dir() {
        state::toggle_dir(session, path);
    }
}
