//! Команды проектов заметок в интерфейсе: подменю «Заметки ▸» в «+» рейла,
//! меню проекта в шапке «Содержимого», экран без открытого проекта,
//! системные диалоги, диалог «Переименовать» и горячие клавиши Ctrl+O /
//! Ctrl+S / Ctrl+Shift+S.
//!
//! Сами операции — в [`super::projects`]; здесь только то, откуда их зовут,
//! и как сообщить об ошибке.

use std::path::{Path, PathBuf};

use rfd::AsyncFileDialog;
use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::input::{Key, Modifiers};
use syngui::prelude::*;
use syngui::widgets::containers::IntoWidget;
use syngui::widgets::feedback::NotificationItem;
use syngui::mgui;
use syngui::widgets::overlay::menu::MenuItem;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::{GestureDetector, TextField, ToolButton};

use crate::components::event_hook::{EventHook, KeyReply};
use crate::context::AppCtx;
use crate::icons::*;
use crate::rail;

use super::project;
use super::projects::{ProjectError, ProjectOpResult};
use super::state::NotesCtx;

const ID_NEW: &str = "notes-project:new";
const ID_OPEN: &str = "notes-project:open";
const ID_SAVE: &str = "notes-project:save";
const ID_SAVE_AS: &str = "notes-project:save-as";
const ID_RENAME: &str = "notes-project:rename";
const ID_REVEAL: &str = "notes-project:reveal";
const ID_CLOSE: &str = "notes-project:close";
const RECENT_PREFIX: &str = "notes-recent:";

// ─── Сообщения ──────────────────────────────────────────────────────────────

pub fn report_error(e: &ProjectError) {
    let app = use_context::<AppCtx>();
    app.notifications.show(NotificationItem::error(tr!("notes.project.error.title")).message(e.message()));
}

/// Итог операции: ошибка — тостом; успех — на страницу заметок.
fn finish(result: ProjectOpResult) {
    match result {
        Ok(()) => rail::navigate("notes"),
        Err(e) => report_error(&e),
    }
}

// ─── Диалоги ────────────────────────────────────────────────────────────────

/// Папка, с которой начинают диалоги: у активного проекта — его, иначе
/// «Документы».
fn dialog_dir(ctx: NotesCtx) -> PathBuf {
    ctx.project_path
        .get_untracked()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| project::resolve_project_path("").parent().map(Path::to_path_buf).unwrap_or_default())
}

fn syn_dialog(ctx: NotesCtx, title: String) -> AsyncFileDialog {
    AsyncFileDialog::new()
        .set_title(title)
        .set_directory(dialog_dir(ctx))
        .add_filter(tr!("notes.project.filter"), &["syn"])
}

pub fn open_dialog() {
    let ctx = use_context::<NotesCtx>();
    let dialog = syn_dialog(ctx, tr!("notes.project.open_title"));
    spawn(async move {
        let Some(file) = dialog.pick_file().await else { return };
        let path = file.path().to_path_buf();
        run_on_main_thread(move || finish(use_context::<NotesCtx>().open_project(&path)));
    });
}

pub fn new_dialog() {
    let ctx = use_context::<NotesCtx>();
    let dialog = syn_dialog(ctx, tr!("notes.project.new_title"))
        .set_file_name(format!("{}.syn", tr!("notes.project.default_name")));
    spawn(async move {
        let Some(file) = dialog.save_file().await else { return };
        let path = file.path().to_path_buf();
        run_on_main_thread(move || finish(use_context::<NotesCtx>().create_project(&path)));
    });
}

pub fn save_as_dialog() {
    let ctx = use_context::<NotesCtx>();
    if !ctx.has_project() {
        return;
    }
    let old_key = rail::notes_key(&ctx.project_path.get_untracked());
    let name = tr!("notes.project.copy_name", title = ctx.project_title.get_untracked());
    let dialog = syn_dialog(ctx, tr!("notes.project.save_as_title")).set_file_name(format!("{name}.syn"));
    spawn(async move {
        let Some(file) = dialog.save_file().await else { return };
        let path = file.path().to_path_buf();
        run_on_main_thread(move || {
            let ctx = use_context::<NotesCtx>();
            match ctx.save_project_as(&path) {
                Ok(()) => {
                    rail::rename_key(&old_key, &rail::notes_key(&ctx.project_path.get_untracked()));
                    use_context::<AppCtx>().notifications.show(
                        NotificationItem::success(tr!("notes.project.saved_as", title = ctx.project_title.get_untracked()))
                            .duration_ms(2500),
                    );
                }
                Err(e) => report_error(&e),
            }
        });
    });
}

pub fn save_now() {
    let ctx = use_context::<NotesCtx>();
    match ctx.save_now() {
        Ok(()) => use_context::<AppCtx>().notifications.show(
            NotificationItem::success(tr!("notes.project.saved", title = ctx.project_title.get_untracked())).duration_ms(1500),
        ),
        Err(ProjectError::NoProject) => {}
        Err(e) => report_error(&e),
    }
}

/// «Закрыть проект» из меню страницы: в отличие от закрытия плитки
/// рейла, со страницы не уходим — последний закрытый проект оставляет
/// экран «создать / открыть».
fn close_active() {
    let ctx = use_context::<NotesCtx>();
    let path = ctx.project_path.get_untracked();
    if path.as_os_str().is_empty() {
        return;
    }
    if let Err(e) = ctx.close_project(&path) {
        report_error(&e);
    }
}

// ─── Переименование ─────────────────────────────────────────────────────────

/// Открыть диалог «Переименовать» для открытого проекта (меню проекта,
/// контекстное меню плитки).
pub fn request_rename(path: &Path) {
    use_context::<NotesCtx>().rename_target.set(Some(path.to_path_buf()));
}

fn rename(path: &Path, name: &str) {
    let ctx = use_context::<NotesCtx>();
    ctx.rename_target.set(None);
    let old_key = rail::notes_key(path);
    match ctx.rename_project(path, name) {
        Ok(dst) if dst != path => {
            rail::rename_key(&old_key, &rail::notes_key(&dst));
            use_context::<AppCtx>().notifications.show(
                NotificationItem::success(tr!("notes.project.renamed", title = project::project_title(&dst))).duration_ms(2000),
            );
        }
        Ok(_) => {}
        Err(e) => report_error(&e),
    }
}

/// Диалог «Переименовать проект». Смонтирован в корне приложения рядом с
/// диалогами закрытия плиток: плитку переименовывают с любого маршрута.
pub fn rename_dialog() -> impl Widget {
    let is_open = use_signal(false);
    create_effect(move || {
        let has = use_context::<NotesCtx>().rename_target.get().is_some();
        if is_open.get_untracked() != has {
            is_open.set(has);
        }
    });
    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .on_close(|| use_context::<NotesCtx>().rename_target.set(None))
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            match use_context::<NotesCtx>().rename_target.get() {
                Some(path) => vec![Box::new(rename_card(path))],
                None => vec![Box::new(DecoratedBox::new().class("code-editor-dialog-empty"))],
            }
        }))
}

fn rename_card(path: PathBuf) -> impl Widget {
    let initial = project::project_title(&path);
    let value = use_signal(initial.clone());
    let submit_path = path.clone();
    let ok_path = path.clone();
    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("notes.project.rename_title")).class("code-editor-dialog-title"),
                    Text::new(tr!("notes.project.rename_hint", path = path.display().to_string())).class("code-editor-dialog-hint"),
                    TextField::new()
                        .text(initial)
                        .placeholder(tr!("notes.project.rename_placeholder"))
                        .autofocus(true)
                        .on_change(move |s| value.set(s.to_string()))
                        .on_submit(move |s| rename(&submit_path, s))
                        .class("code-editor-dialog-input"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(|| use_context::<NotesCtx>().rename_target.set(None))
                                .class("code-editor-dialog-btn-secondary"),
                            Button::new(tr!("app.ok"))
                                .leading_icon(MI_CHECK)
                                .on_click(move || rename(&ok_path, &value.get_untracked()))
                                .class("code-editor-dialog-btn-primary"),
                        ],
                ]
        ]
    }
}

// ─── Меню ───────────────────────────────────────────────────────────────────

/// Подпись недавнего проекта: имя файла, а при совпадении имён — ещё и
/// папка («Заметки — Работа»).
fn recent_items(ctx: NotesCtx) -> Vec<MenuItem> {
    let recent = ctx.recent_closed();
    recent
        .iter()
        .map(|path| {
            let title = project::project_title(path);
            let clash = recent.iter().filter(|p| project::project_title(p) == title).count() > 1;
            let label = match path.parent().and_then(|p| p.file_name()).filter(|_| clash) {
                Some(dir) => format!("{title} — {}", dir.to_string_lossy()),
                None => title,
            };
            MenuItem::new(format!("{RECENT_PREFIX}{}", path.display()), label).icon(MI_HISTORY)
        })
        .collect()
}

/// Пункты подменю «Заметки ▸» в «+» рейла. Читает недавние через `.get()` —
/// рейл перестраивает меню, когда список меняется.
pub fn add_menu_items() -> Vec<MenuItem> {
    let ctx = use_context::<NotesCtx>();
    let mut items = vec![
        MenuItem::new(ID_NEW, tr!("notes.project.new")).icon(MI_NOTE_ADD),
        MenuItem::new(ID_OPEN, tr!("notes.project.open")).icon(MI_FOLDER_OPEN).shortcut("Ctrl+O"),
    ];
    let recent = recent_items(ctx);
    if !recent.is_empty() {
        items.push(MenuItem::separator());
        items.extend(recent);
    }
    items
}

/// Меню проекта в шапке «Содержимого».
fn project_menu_items(ctx: NotesCtx) -> Vec<MenuItem> {
    let has = !ctx.project_path.get().as_os_str().is_empty();
    let recent = recent_items(ctx);
    let mut items = vec![
        MenuItem::new(ID_NEW, tr!("notes.project.new")).icon(MI_NOTE_ADD),
        MenuItem::new(ID_OPEN, tr!("notes.project.open")).icon(MI_FOLDER_OPEN).shortcut("Ctrl+O"),
    ];
    if !recent.is_empty() {
        items.push(MenuItem::new("notes-project:recent", tr!("notes.project.recent")).icon(MI_HISTORY).children(recent));
    }
    items.extend([
        MenuItem::separator(),
        MenuItem::new(ID_SAVE, tr!("notes.project.save")).icon(MI_SAVE).shortcut("Ctrl+S").disabled(!has),
        MenuItem::new(ID_SAVE_AS, tr!("notes.project.save_as")).icon(MI_SAVE_AS).shortcut("Ctrl+Shift+S").disabled(!has),
        MenuItem::new(ID_RENAME, tr!("notes.project.rename")).icon(MI_DRIVE_FILE_RENAME_OUTLINE).disabled(!has),
        MenuItem::new(ID_REVEAL, tr!("notes.project.reveal")).icon(MI_FOLDER).disabled(!has),
        MenuItem::separator(),
        MenuItem::new(ID_CLOSE, tr!("notes.project.close")).icon(MI_CLOSE).disabled(!has),
    ]);
    items
}

/// Выполнить пункт меню проектов. `false` — пункт не отсюда.
pub fn handle_menu(id: &str) -> bool {
    if let Some(path) = id.strip_prefix(RECENT_PREFIX) {
        finish(use_context::<NotesCtx>().open_project(Path::new(path)));
        return true;
    }
    match id {
        ID_NEW => new_dialog(),
        ID_OPEN => open_dialog(),
        ID_SAVE => save_now(),
        ID_SAVE_AS => save_as_dialog(),
        ID_RENAME => {
            let path = use_context::<NotesCtx>().project_path.get_untracked();
            if !path.as_os_str().is_empty() {
                request_rename(&path);
            }
        }
        ID_REVEAL => {
            let path = use_context::<NotesCtx>().project_path.get_untracked();
            if path.is_file() {
                crate::pages::code_editor::fs_actions::reveal_in_files(&path);
            }
        }
        ID_CLOSE => close_active(),
        _ => return false,
    }
    true
}

/// Кнопка меню проекта (шапка «Содержимого»). Пункты читают проекты и
/// недавние через `.get()` — шапка пересобирается, когда они меняются.
pub fn menu_button() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    let open = use_signal(false);
    let pos = use_signal(Point::zero());
    let btn = ToolButton::new(MI_FOLDER_OPEN)
        .tooltip(tr!("notes.project.menu"))
        .on_click_with_bounds(move |_, bounds| {
            pos.set(Point::new(bounds.origin.x, bounds.origin.y + bounds.size.height + 4.0));
            open.set(true);
        })
        .class("panel-header-action");
    let menu = PopupMenu::new()
        .items(project_menu_items(ctx))
        .is_open(open)
        .position(pos)
        .on_select(|id| {
            handle_menu(id);
        });
    Stack::new().clip(false).child(btn).child(menu)
}

// ─── Экран без проекта ──────────────────────────────────────────────────────

/// Центр страницы, когда ни один проект не открыт: создать, открыть,
/// недавние.
pub fn start_screen() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    let mut col = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(MI_EDIT_NOTE).class("notes-empty-icon"))
        .child(Text::new(tr!("notes.project.none.title")).class("notes-empty-title"))
        .child(Text::new(tr!("notes.project.none.hint")).class("notes-empty-hint"))
        .child(
            Row::new()
                .gap(10.0)
                .child(
                    Button::new(tr!("notes.project.new"))
                        .leading_icon(MI_NOTE_ADD)
                        .on_click(new_dialog)
                        .class("code-editor-dialog-btn-primary"),
                )
                .child(
                    Button::new(tr!("notes.project.open"))
                        .leading_icon(MI_FOLDER_OPEN)
                        .on_click(open_dialog)
                        .class("code-editor-dialog-btn-secondary"),
                ),
        );
    let recent = ctx.recent_closed();
    if !recent.is_empty() {
        let mut list = Column::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("notes-start-recent")
            .child(Text::new(tr!("notes.project.recent")).class("notes-start-recent-title"));
        for path in recent {
            let target = path.clone();
            list = list.child(
                GestureDetector::new()
                    .cursor(syngui::input::CursorIcon::Pointer)
                    .on_click(move || finish(use_context::<NotesCtx>().open_project(&target)))
                    .child(
                        Row::new()
                            .gap(8.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .class("notes-start-recent-row")
                            .child(Icon::new(MI_HISTORY).class("notes-start-recent-icon"))
                            .child(Text::new(project::project_title(&path)).max_lines(1).class("notes-start-recent-name"))
                            .child(
                                DecoratedBox::new().class("grow").child(
                                    Text::new(path.parent().map(|p| p.display().to_string()).unwrap_or_default())
                                        .max_lines(1)
                                        .class("notes-start-recent-path"),
                                ),
                            ),
                    ),
            );
        }
        col = col.child(list);
    }
    Center::new().child(col)
}

// ─── Горячие клавиши ────────────────────────────────────────────────────────

/// Хоткеи проектов на странице заметок. Обёртка всей оболочки, как у
/// поиска: клавиша доходит сюда и без фокуса внутри страницы.
pub fn hotkey_scope<M>(child: impl IntoWidget<M>) -> impl Widget {
    EventHook::new()
        .on_key_down(|key: Key, mods: Modifiers| {
            if !mods.ctrl || mods.alt {
                return KeyReply::Ignore;
            }
            if use_context::<AppCtx>().current_route.get_untracked() != "notes" {
                return KeyReply::Ignore;
            }
            match key {
                Key::O if !mods.shift => open_dialog(),
                Key::S if mods.shift => save_as_dialog(),
                Key::S => save_now(),
                _ => return KeyReply::Ignore,
            }
            KeyReply::Handled
        })
        .child(child)
}
