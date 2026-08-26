//! Правая панель — список открытых файлов активной сессии.
//!
//! Reactive(ListView) с кастомным `item_widget`: иконка + имя файла +
//! dirty-точка/X-кнопка. Активный файл выделяется selection'ом.
//!
//! Подписки: `open_files` (структура списка), `active_file` (selection),
//! `file_contents` + `disk_contents` (dirty-индикатор по каждому файлу).

use std::path::PathBuf;
use std::sync::Arc;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::{ListItem, ListView, SelectionMode};

use syngui::widgets::overlay::{Draggable, DropArea};

use crate::icons::{MI_CLOSE, MI_DESCRIPTION};

use super::file_icons;
use super::state::{self, is_dirty, CodeEditorCtx, CodeSession};

/// `drag_type` для перетаскивания вкладок открытых файлов. Уникален для
/// этого виджета, чтобы DropArea не реагировала на drop'ы из других мест
/// (TreeView в будущем мог бы получать external drag).
const DRAG_TYPE_TAB: &str = "code-editor-tab";

/// Переставить `src` непосредственно перед `target` в `open_files`.
/// `src == target` или путь не найден — no-op. Реализация: сначала
/// удаляем `src` из Vec, потом вставляем перед текущим положением
/// `target` (с учётом сдвига после удаления).
fn reorder_open_files(session: CodeSession, src: PathBuf, target: PathBuf) {
    if src == target {
        return;
    }
    session.open_files.update(|v| {
        let Some(src_idx) = v.iter().position(|p| p == &src) else {
            return;
        };
        // Удаляем src.
        v.remove(src_idx);
        // Находим target после удаления (индекс мог сдвинуться).
        let target_idx = match v.iter().position(|p| p == &target) {
            Some(i) => i,
            None => {
                // target исчез (вряд ли — мы только что были на нём в drop'е) —
                // вставляем src обратно в конец.
                v.push(src);
                return;
            }
        };
        v.insert(target_idx, src);
    });
}

pub fn view() -> impl Widget {
    DecoratedBox::new().class("code-editor-files-panel").child(mgui! {
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                header(),
                body(),
            ]
    })
}

fn header() -> impl Widget {
    DecoratedBox::new().class("code-editor-files-header").child(move || {
        let code = use_context::<CodeEditorCtx>();
        let count = code
            .active_session()
            .map(|s| s.open_files.get().len())
            .unwrap_or(0);
        let label = if count == 0 {
            tr!("code.open_files.header.title")
        } else {
            tr!("code.open_files.header.title_count", count = count.to_string())
        };
        Text::new(label).class("code-editor-files-title")
    })
}

fn body() -> impl Widget {
    DecoratedBox::new().class("code-editor-files-body grow").child(move || {
        let code = use_context::<CodeEditorCtx>();
        let child: Box<dyn Widget> = match code.active_session() {
            None => Box::new(empty_state()),
            Some(session) => {
                let files = session.open_files.get();
                let active = session.active_file.get();
                let contents = session.file_contents.get();
                let disk = session.disk_contents.get();
                if files.is_empty() {
                    Box::new(empty_state())
                } else {
                    list_widget(session, files, active, contents, disk)
                }
            }
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    })
}

fn empty_state() -> impl Widget {
    Center::new().child(mgui! {
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_DESCRIPTION).class("code-editor-files-empty-icon"),
                Text::new(tr!("code.open_files.empty.title")).class("code-editor-files-empty-title"),
                Text::new(tr!("code.open_files.empty.hint")).class("code-editor-files-empty-hint"),
            ]
    })
}

fn list_widget(
    session: CodeSession,
    files: Vec<PathBuf>,
    active: Option<PathBuf>,
    contents: std::collections::HashMap<PathBuf, String>,
    disk: std::collections::HashMap<PathBuf, String>,
) -> Box<dyn Widget> {
    // ListView::item_widget билдер должен быть `Send + Sync + 'static`.
    // Перекладываем все нужные данные в Arc'ы. Снимок реактивно
    // пересоздаётся при изменении сигналов (вся list_widget пересобирается
    // из Reactive-замыкания родителя).
    let files_arc: Arc<Vec<PathBuf>> = Arc::new(files);
    let contents_arc = Arc::new(contents);
    let disk_arc = Arc::new(disk);

    let items: Vec<ListItem> = files_arc
        .iter()
        .map(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string());
            ListItem::new(name)
        })
        .collect();

    let selected_idx = active
        .as_ref()
        .and_then(|a| files_arc.iter().position(|p| p == a))
        .map(|i| vec![i])
        .unwrap_or_default();

    let files_for_select = files_arc.clone();
    let files_for_widget = files_arc.clone();
    let contents_for_widget = contents_arc;
    let disk_for_widget = disk_arc;

    Box::new(
        ListView::new(items)
            .item_height(40.0)
            .selection_mode(SelectionMode::Single)
            .selected(selected_idx)
            .on_select(move |idx| {
                if let Some(path) = files_for_select.get(idx).cloned() {
                    if session.active_file.get_untracked().as_ref() != Some(&path) {
                        session.active_file.set(Some(path));
                        session.editor_gen.update(|n| *n = n.wrapping_add(1));
                    }
                }
            })
            .item_widget(move |idx, _item, selected, _hovered| {
                let path = match files_for_widget.get(idx).cloned() {
                    Some(p) => p,
                    None => {
                        return Box::new(DecoratedBox::new().class("code-editor-files-item"))
                            as Box<dyn Widget>;
                    }
                };
                let dirty = is_dirty(&contents_for_widget, &disk_for_widget, &path);
                let row_class = if selected {
                    "code-editor-files-item selected"
                } else {
                    "code-editor-files-item"
                };
                let dirty_class = if dirty {
                    "code-editor-files-item-dirty active"
                } else {
                    "code-editor-files-item-dirty"
                };
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());

                let close_path = path.clone();
                let icon_codepoint = file_icons::icon_for_path(&path, false, false);
                let icon_class = format!(
                    "code-editor-files-item-icon {}",
                    file_icons::class_for_path(&path, false, false)
                );

                let row = mgui! {
                    DecoratedBox::new().class(row_class) => [
                        Padding::symmetric(8.0, 6.0) => [
                            Row::new()
                                .gap(8.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center) => [
                                    Icon::new(icon_codepoint).class(icon_class),
                                    DecoratedBox::new().class("grow").child(
                                        Text::new(name).class("code-editor-files-item-name")
                                    ),
                                    DecoratedBox::new().class(dirty_class),
                                    ToolButton::new(MI_CLOSE)
                                        .tooltip(tr!("code.open_files.item.close_tooltip"))
                                        .on_click(move || {
                                            state::close_file(session, close_path.clone());
                                        })
                                        .class("code-editor-files-item-close"),
                                ]
                        ]
                    ]
                };

                // Drag-and-drop reordering. Draggable передаёт path в payload,
                // DropArea принимает только из той же группы (DRAG_TYPE_TAB)
                // и переупорядочивает open_files. Двойная обёртка
                // Draggable→DropArea — оба слоя нужны, чтобы один и тот же
                // item был и источником, и целью.
                //
                // Draggable.handle_event поглощает MouseDown, поэтому
                // `ListView::on_select` не срабатывает. Переключение активного
                // файла перенесено в `Draggable::on_click` — тот же callback
                // отрабатывает только если drag не стартовал (см. логику в
                // syngui/widgets/overlay/draggable.rs `MouseUp`).
                let drag_payload = path.to_string_lossy().to_string();
                let drop_target = path.clone();
                let click_path = path.clone();
                Box::new(
                    Draggable::new(DRAG_TYPE_TAB, drag_payload)
                        .on_click(move || {
                            if session.active_file.get_untracked().as_ref() != Some(&click_path) {
                                session.active_file.set(Some(click_path.clone()));
                                session.editor_gen.update(|n| *n = n.wrapping_add(1));
                            }
                        })
                        .child(
                            DropArea::new()
                                .accept_types(vec![DRAG_TYPE_TAB.to_string()])
                                .on_drop(move |data| {
                                    let src = std::path::PathBuf::from(&data.payload);
                                    reorder_open_files(session, src, drop_target.clone());
                                })
                                .child(row),
                        ),
                ) as Box<dyn Widget>
            })
            .class("code-editor-files-list"),
    )
}
