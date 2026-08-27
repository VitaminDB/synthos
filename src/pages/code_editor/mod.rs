//! Страница «Редактор кода» в стиле Zed/VSCode (multi-session).
//!
//! Маршрут `code`. Контекст [`state::CodeEditorCtx`] провайдится единожды в
//! [`crate::run_desktop`] и достаётся компонентами через
//! `use_context::<CodeEditorCtx>()`. Активная сессия отслеживается отдельно
//! через `code.active_id` и сигнал `code.session_gen`; при их изменении
//! Reactive-обёртки в дочерних panel'ях пересобираются.
//!
//! ```text
//! workspace_frame [
//!   page_header                         ─── папка сессии + путь, поиск, тогглы панелей
//!   SplitView(Horizontal, session.left_split_ratio)        (только при наличии активной сессии)
//!   ├── file_tree                        ─── header(folder + open_btn) + Reactive(TreeView)
//!   └── SplitView(Horizontal, session.right_split_ratio)
//!       ├── center                       ─── вертикальный SplitView(editor↔terminal)
//!       │   └── SplitView(Vertical, session.split_ratio)
//!       │       ├── editor_pane          ─── header(filename + save) + Reactive(CodeEditor)
//!       │       └── terminal_pane        ─── tabs + Reactive(Terminal::attach)
//!       └── open_files                   ─── header(счётчик) + Reactive(ListView)
//! ]
//! ```
//!
//! Размеры splitter'ов индивидуальны на каждую сессию (`split_ratio`, `left_split_ratio`,
//! `right_split_ratio` в [`state::CodeSession`]); видимость боковых панелей — общая для
//! страницы (`AppCtx.panels.code`). Drag пишет в сигнал активной сессии;
//! `install_config_autosave` сериализует значения в `code_sessions[i]` элементы
//! `~/.config/synthos/config.json`. Reactive-обёртка пересоздаёт каркас при смене
//! `code.session_gen` / `code.active_id` — переключение между сессиями подменяет
//! ratio_signal на свежий, и layout мгновенно становится «под эту сессию».
//!
//! Когда `code.active_session().is_none()` (сессий нет вовсе) — вместо каркаса
//! рендерится no-session placeholder с подсказкой создать сессию через `+`.

pub mod dialogs;
pub mod drafts;
pub mod editor_pane;
pub mod file_icons;
pub mod file_tree;
pub mod fs_actions;
pub mod fs_ops;
pub mod fs_watcher;
pub mod git_status;
pub mod open_files;
pub mod state;
pub mod terminal_activity;
pub mod terminal_pane;
pub mod text_diff;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::{SplitDirection, SplitView};

use crate::components::page_header::{self, HeaderSpec};
use crate::components::workspace_frame::{self, expand, FrameSpec, Pane};
use crate::context::AppCtx;
use crate::icons::{MI_CODE, MI_DESCRIPTION, MI_FOLDER};

pub use state::CodeEditorCtx;

pub fn view() -> impl Widget {
    let main = DecoratedBox::new()
        .class("code-editor-page")
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let app = use_context::<AppCtx>();
            let code = use_context::<CodeEditorCtx>();
            // Подписки на структурные сигналы — пересборка UI при switch/close/create.
            let _ = code.session_gen.get();
            let _ = code.active_id.get();

            let Some(session) = code.active_session() else {
                return vec![Box::new(workspace_frame::view(
                    header(None),
                    FrameSpec::new("code-editor-h-split", || Box::new(no_session_placeholder())),
                ))];
            };

            // Lazy-init первого таба терминала в активной сессии. Guard через
            // `tabs.is_empty()` — повторные вызовы view() уже видят таб и не
            // дублируют. `tabs.update` внутри `add_terminal` триггерит ещё один
            // ребилд, но guard гасит цикл (на втором заходе tab'ы не пусты).
            if session.terminals.tabs.get_untracked().is_empty() {
                let _ = state::add_terminal(session, app.clone());
            }

            let (left_visible, right_visible) = app.panels.code;
            let spec = FrameSpec::new("code-editor-h-split", move || Box::new(center(session)))
                .left(Pane::new(left_visible, session.left_split_ratio, 160.0, || {
                    Box::new(file_tree::view())
                }))
                .right(Pane::new(right_visible, session.right_split_ratio, 160.0, || {
                    Box::new(open_files::view())
                }));
            vec![Box::new(workspace_frame::view(header(Some(session)), spec))]
        }));

    // Раньше здесь был локальный Snackbar в Stack'е; миграция на
    // глобальный `AppCtx::notifications` (TopRight) — все feedback'и
    // приходят через единый NotificationHost, который смонтирован один раз
    // в `lib.rs`. См. также TASK.md «Snackbar refactor».
    Stack::new()
        .fit(StackFit::Expand)
        .children(vec![Box::new(main) as Box<dyn Widget>, Box::new(dialogs::view())])
}

/// Центр: вертикальный split editor↔terminal.
fn center(session: state::CodeSession) -> impl Widget {
    DecoratedBox::new().class("code-editor-center grow").child(
        SplitView::new(editor_pane::view(), terminal_pane::view())
            .class("code-editor-split")
            .direction(SplitDirection::Vertical)
            .ratio_signal(session.split_ratio)
            .min_size(80.0)
            .divider_width(6.0),
    )
}

/// Общая шапка: имя папки сессии + путь к корню. Кнопки «открыть папку» и
/// «сохранить» остаются в панелях рядом с деревом и файлом.
fn header(session: Option<state::CodeSession>) -> impl Widget {
    let app = use_context::<AppCtx>();
    let (left_visible, right_visible) = app.panels.code;
    let identity: Box<dyn Widget> = Box::new(DecoratedBox::new().child(move || {
        let code = use_context::<CodeEditorCtx>();
        let _ = code.session_gen.get();
        let folder = session.and_then(|s| s.root_folder.get());
        let (icon, title, subtitle) = match (&session, &folder) {
            (Some(_), Some(path)) => (
                MI_FOLDER,
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string()),
                path.display().to_string(),
            ),
            (Some(s), None) => {
                let idx = code
                    .sessions
                    .get_untracked()
                    .iter()
                    .position(|x| x.id == s.id)
                    .unwrap_or(0);
                (
                    MI_DESCRIPTION,
                    tr!("nav.session.unnamed", n = idx + 1),
                    tr!("code.header.no_folder"),
                )
            }
            (None, _) => (
                MI_CODE,
                tr!("search.page.code"),
                tr!("code.mod.no_session.title"),
            ),
        };
        expand(page_header::identity_text(icon, title, subtitle))
    }));
    let toggles = if session.is_some() {
        (Some(left_visible), Some(right_visible))
    } else {
        (None, None)
    };
    page_header::view(HeaderSpec::new(identity).toggles(toggles.0, toggles.1))
}

/// Пустой стейт страницы — нет ни одной сессии. Кликабельных action'ов
/// здесь нет: создание сессии живёт в рейле (меню «+»).
fn no_session_placeholder() -> impl Widget {
    Center::new().child(mgui! {
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_CODE).class("code-editor-no-session-icon"),
                Text::new(tr!("code.mod.no_session.title")).class("code-editor-no-session-title"),
                Text::new(tr!("code.mod.no_session.hint"))
                    .class("code-editor-no-session-hint"),
            ]
    })
}
