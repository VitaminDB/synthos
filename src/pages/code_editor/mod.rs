//! Страница «Редактор кода» в стиле Zed/VSCode (multi-session).
//!
//! Маршрут `code`. Контекст [`state::CodeEditorCtx`] провайдится единожды в
//! [`crate::run_desktop`] и достаётся компонентами через
//! `use_context::<CodeEditorCtx>()`. Активная сессия отслеживается отдельно
//! через `code.active_id` и сигнал `code.session_gen`; при их изменении
//! Reactive-обёртки в дочерних panel'ях пересобираются.
//!
//! ```text
//! workspace_frame                                          (только при наличии активной сессии)
//!   [▤ папка · путь  ⎘] [файл · каталог   🔍 поиск   ↵ ⟲ 💾] [Открытые файлы · N ▥]   ← заголовки
//!   ├── file_tree                        ─── Reactive(TreeView)
//!   └── SplitView(Horizontal, session.right_split_ratio)
//!       ├── center                       ─── Reactive(session.editor_visible)
//!       │   └── SplitView(Vertical, session.split_ratio)   ← редактор показан
//!       │       ├── editor_pane          ─── Reactive(CodeEditor)
//!       │       └── terminal_pane        ─── tabs + Reactive(Terminal::attach)
//!       │   └── terminal_pane                               ← редактор скрыт
//!       └── open_files                   ─── Reactive(ListView)
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

use crate::components::panel_header::{self, CenterSpec};
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
                return vec![Box::new(workspace_frame::view(FrameSpec::new(
                    "code-editor-h-split",
                    || {
                        Box::new(panel_header::center(CenterSpec::new(
                            panel_header::identity_text(
                                MI_CODE,
                                tr!("search.page.code"),
                                tr!("code.mod.no_session.title"),
                            ),
                        )))
                    },
                    || Box::new(no_session_placeholder()),
                )))];
            };

            // Терминал не заводится сам: PTY поднимает shell, а тот —
            // профиль пользователя со всем, что в нём прописано. Открытие
            // страницы такого не просило; первый таб появляется по «+» в
            // панели терминала (до этого там подсказка `empty_placeholder`).

            let (left_visible, right_visible) = app.panels.code;
            let spec = FrameSpec::new(
                "code-editor-h-split",
                || Box::new(center_header()),
                move || Box::new(center(session)),
            )
            .left(Pane::new(
                left_visible,
                session.left_split_ratio,
                160.0,
                move || Box::new(left_header(session)),
                || Box::new(file_tree::view()),
            ))
            .right(Pane::new(
                right_visible,
                session.right_split_ratio,
                160.0,
                || Box::new(open_files::header()),
                || Box::new(open_files::view()),
            ));
            vec![Box::new(workspace_frame::view(spec))]
        }));

    // Раньше здесь был локальный Snackbar в Stack'е; миграция на
    // глобальный `AppCtx::notifications` (TopRight) — все feedback'и
    // приходят через единый NotificationHost, который смонтирован один раз
    // в `lib.rs`. См. также TASK.md «Snackbar refactor».
    Stack::new()
        .fit(StackFit::Expand)
        .children(vec![Box::new(main) as Box<dyn Widget>, Box::new(dialogs::view())])
}

/// Центр: вертикальный split editor↔terminal, а при скрытом редакторе
/// (`session.editor_visible`) — один терминал на всю высоту. Подписка своя,
/// чтобы переключение не пересобирало каркас вместе с деревом файлов.
fn center(session: state::CodeSession) -> impl Widget {
    DecoratedBox::new().class("code-editor-center grow").child(move || {
        let child: Box<dyn Widget> = if session.editor_visible.get() {
            Box::new(
                SplitView::new(editor_pane::view(), terminal_pane::view(false))
                    .class("code-editor-split")
                    .direction(SplitDirection::Vertical)
                    .ratio_signal(session.split_ratio)
                    .min_size(80.0)
                    .divider_width(6.0),
            )
        } else {
            Box::new(terminal_pane::view(true))
        };
        expand(child)
    })
}

/// Заголовок левой панели: папка сессии + путь к корню, справа — кнопка
/// «Открыть папку».
fn left_header(session: state::CodeSession) -> impl Widget {
    let identity = DecoratedBox::new().class("grow").child(move || {
        let code = use_context::<CodeEditorCtx>();
        let _ = code.session_gen.get();
        // Путь к корню — подзаголовок с сжатием середины: в узкой панели
        // `/home/master/Projects/2027/synthos` превращается в
        // `~/…/2027/synthos`, полный путь остаётся в tooltip.
        let identity = match session.root_folder.get() {
            Some(path) => {
                let title = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                panel_header::identity_path(MI_FOLDER, title, &path)
            }
            None => {
                let idx = code
                    .sessions
                    .get_untracked()
                    .iter()
                    .position(|x| x.id == session.id)
                    .unwrap_or(0);
                panel_header::identity_text(
                    MI_DESCRIPTION,
                    tr!("nav.session.unnamed", n = idx + 1),
                    tr!("code.header.no_folder"),
                )
            }
        };
        expand(identity)
    });
    mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            identity,
            file_tree::open_folder_button(),
        ]
    }
}

/// Центральный заголовок: активный файл + поиск + действия над файлом.
fn center_header() -> impl Widget {
    panel_header::center(
        CenterSpec::new(Box::new(editor_pane::header_identity()))
            .actions(editor_pane::header_actions()),
    )
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
