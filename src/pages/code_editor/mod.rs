//! Страница «Редактор кода» в стиле Zed/VSCode (multi-session).
//!
//! Маршрут `code`. Контекст [`state::CodeEditorCtx`] провайдится единожды в
//! [`crate::run_desktop`] и достаётся компонентами через
//! `use_context::<CodeEditorCtx>()`. Активная сессия отслеживается отдельно
//! через `code.active_id` и сигнал `code.session_gen`; при их изменении
//! Reactive-обёртки в дочерних panel'ях пересобираются.
//!
//! ```text
//! SplitView(Horizontal, session.left_split_ratio)        (только при наличии активной сессии)
//! ├── file_tree                              ─── header(folder + open_btn) + Reactive(TreeView)
//! └── SplitView(Horizontal, session.right_split_ratio)
//!     ├── center                             ─── вертикальный SplitView(editor↔terminal)
//!     │   └── SplitView(Vertical, session.split_ratio)
//!     │       ├── editor_pane                ─── header(filename + save) + Reactive(CodeEditor)
//!     │       └── terminal_pane              ─── tabs + Reactive(Terminal::attach)
//!     └── open_files                         ─── header(счётчик) + Reactive(ListView)
//! ```
//!
//! Размеры splitter'ов индивидуальны на каждую сессию (`split_ratio`, `left_split_ratio`,
//! `right_split_ratio` в [`state::CodeSession`]). Drag пишет в сигнал активной сессии;
//! `install_config_autosave` сериализует значения в `code_sessions[i]` элементы
//! `~/.config/synthos/config.json`. Reactive-обёртка пересоздаёт SplitView при смене
//! `code.session_gen` / `code.active_id` — переключение между сессиями подменяет
//! ratio_signal на свежий, и layout мгновенно становится «под эту сессию».
//!
//! Когда `code.active_session().is_none()` (сессий нет вовсе) — вместо Row
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
pub mod terminal_pane;
pub mod text_diff;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::{SplitDirection, SplitView};

use crate::context::AppCtx;
use crate::icons::MI_CODE;

pub use state::CodeEditorCtx;

pub fn view() -> impl Widget {
    // Snackbar для уведомлений (file too large, не текстовый файл, ошибки IO).
    // Текст обновляется через `code.show_notice(...)`, который и поднимает флаг.
    // Snackbar — оверлей: занимает 0px в layout, рендерится поверх Stack'ом.
    let main = DecoratedBox::new()
        .class("code-editor-page")
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let app = use_context::<AppCtx>();
            let code = use_context::<CodeEditorCtx>();
            // Подписки на структурные сигналы — пересборка UI при switch/close/create.
            let _ = code.session_gen.get();
            let _ = code.active_id.get();

            let Some(session) = code.active_session() else {
                return vec![Box::new(no_session_placeholder())];
            };

            // Lazy-init первого таба терминала в активной сессии. Guard через
            // `tabs.is_empty()` — повторные вызовы view() уже видят таб и не
            // дублируют. `tabs.update` внутри `add_terminal` триггерит ещё один
            // ребилд, но guard гасит цикл (на втором заходе tab'ы не пусты).
            if session.terminals.tabs.get_untracked().is_empty() {
                let _ = state::add_terminal(session, app.clone());
            }

            // Per-session splitter ratio'ы — берём прямо из активной сессии.
            // При переключении сессий `code.active_id` меняется → Reactive
            // пересоздаёт SplitView'ы с новыми сигналами; layout мгновенно
            // подстраивается под выбранную сессию.
            let ratio_signal = session.split_ratio;
            let left_ratio = session.left_split_ratio;
            let right_ratio = session.right_split_ratio;

            // Центр: вертикальный split editor↔terminal. Идентичен прежнему
            // рендеру, только теперь обёрнут двумя горизонтальными split'ами.
            let center = DecoratedBox::new().class("code-editor-center grow").child(
                SplitView::new(editor_pane::view(), terminal_pane::view())
                    .class("code-editor-split")
                    .direction(SplitDirection::Vertical)
                    .ratio_signal(ratio_signal)
                    .min_size(80.0)
                    .divider_width(6.0),
            );

            // Правый горизонтальный split: center | open_files.
            // ratio пишется в `session.right_split_ratio` при drag.
            let center_with_files = SplitView::new(center, open_files::view())
                .class("code-editor-h-split")
                .direction(SplitDirection::Horizontal)
                .ratio_signal(right_ratio)
                .min_size(160.0)
                .divider_width(6.0);

            // Левый горизонтальный split: file_tree | (center+open_files).
            // ratio пишется в `session.left_split_ratio` при drag.
            vec![Box::new(
                SplitView::new(file_tree::view(), center_with_files)
                    .class("code-editor-h-split")
                    .direction(SplitDirection::Horizontal)
                    .ratio_signal(left_ratio)
                    .min_size(160.0)
                    .divider_width(6.0),
            )]
        }));

    // Раньше здесь был локальный Snackbar в Stack'е; миграция на
    // глобальный `AppCtx::notifications` (TopRight) — все feedback'и
    // приходят через единый NotificationHost, который смонтирован один раз
    // в `lib.rs`. См. также TASK.md «Snackbar refactor».
    Stack::new()
        .fit(StackFit::Expand)
        .children(vec![Box::new(main) as Box<dyn Widget>, Box::new(dialogs::view())])
}

/// Пустой стейт страницы — нет ни одной сессии. Кликабельных action'ов
/// здесь нет: создание сессии живёт в sidebar (кнопка `+`).
fn no_session_placeholder() -> impl Widget {
    Center::new().child(mgui! {
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_CODE).class("code-editor-no-session-icon"),
                Text::new("Нет открытых сессий").class("code-editor-no-session-title"),
                Text::new("Нажмите «+» в боковой панели слева, чтобы создать сессию")
                    .class("code-editor-no-session-hint"),
            ]
    })
}
