//! Подтверждение выхода: что останется недоделанным, если закрыть окно.
//!
//! Guard стоит в `AppBuilder::on_close_request` (см. `lib.rs`): системный
//! запрос закрытия (✕ в шапке, Alt+F4, «Выход» из трея) сперва зовёт
//! [`allow_close`]. Если в приложении что-то живо — работающая команда в
//! терминале, ход агента, несохранённый файл или граф, идущая загрузка —
//! guard возвращает `false` и взводит [`QuitCtx::open`]; окно остаётся, а
//! этот Portal показывает список причин.
//!
//! «Выйти» ставит [`QuitCtx::force`] и просит закрытие ещё раз — второй
//! проход guard'а пропускает его, предварительно дописав на диск всё, что
//! ждало автосейва. Флаг живёт до конца процесса: отменить выход после
//! подтверждения уже нельзя.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::{PortalAnchor, WindowControl};

use crate::icons::*;
use crate::pages::code_editor::state::{self, CodeEditorCtx};
use crate::pages::huggingface::state::{DlStatus, HuggingFaceCtx};
use crate::pages::node_editor::tabs::EditorWorkspace;
use crate::syn_chat::state::SynChatCtx;

/// Состояние диалога. Провайдится один раз на старте (`lib.rs`).
#[derive(Clone, Copy)]
pub struct QuitCtx {
    /// Диалог показан.
    pub open: RwSignal<bool>,
    /// Причины: строки для списка в карточке.
    pub reasons: RwSignal<Vec<String>>,
    /// Выход подтверждён — guard больше не мешает.
    pub force: RwSignal<bool>,
}

impl QuitCtx {
    pub fn new() -> Self {
        Self {
            open: use_signal(false),
            reasons: use_signal(Vec::new()),
            force: use_signal(false),
        }
    }
}

impl Default for QuitCtx {
    fn default() -> Self {
        Self::new()
    }
}

/// Guard закрытия окна. Вызывается syngui на главном потоке.
pub fn allow_close() -> bool {
    let quit = use_context::<QuitCtx>();
    if quit.force.get_untracked() {
        flush_pending_writes();
        return true;
    }
    let reasons = pending_work();
    if reasons.is_empty() {
        flush_pending_writes();
        return true;
    }
    quit.reasons.set(reasons);
    quit.open.set(true);
    false
}

/// Дописать на диск то, что ждало автосейва по дебаунсу.
fn flush_pending_writes() {
    crate::pages::notes::autosave::flush_all();
    crate::syn_chat::autosave::flush_for_exit();
}

/// Причины, по которым закрытие стоит подтвердить. Пустой список — можно
/// выходить молча.
fn pending_work() -> Vec<String> {
    let mut out = Vec::new();

    // Терминалы с живой foreground-группой: закрытие убьёт то, что в них
    // запущено (сборка, ssh, TUI).
    let code = use_context::<CodeEditorCtx>();
    let mut busy_terminals = 0usize;
    let mut dirty_files = 0usize;
    for session in code.sessions.get_untracked() {
        for tab in session.terminals.tabs.get_untracked() {
            if tab.session.is_busy() {
                busy_terminals += 1;
            }
        }
        let contents = session.file_contents.get_untracked();
        let disk = session.disk_contents.get_untracked();
        dirty_files += session
            .open_files
            .get_untracked()
            .iter()
            .filter(|p| state::is_dirty(&contents, &disk, p))
            .count();
    }
    if busy_terminals > 0 {
        out.push(trn!("app.quit.reason.terminals", busy_terminals));
    }
    if dirty_files > 0 {
        out.push(trn!("app.quit.reason.files", dirty_files));
    }

    if use_context::<SynChatCtx>().pending.get_untracked() {
        out.push(tr!("app.quit.reason.agent"));
    }

    let dirty_graphs = use_context::<EditorWorkspace>()
        .tabs
        .get_untracked()
        .iter()
        .filter(|t| t.dirty.get_untracked())
        .count();
    if dirty_graphs > 0 {
        out.push(trn!("app.quit.reason.graphs", dirty_graphs));
    }

    let downloads = use_context::<HuggingFaceCtx>()
        .downloads
        .get_untracked()
        .values()
        .filter(|d| matches!(d.status, DlStatus::Active | DlStatus::Pending))
        .count();
    if downloads > 0 {
        out.push(trn!("app.quit.reason.downloads", downloads));
    }

    out
}

pub fn view() -> impl Widget {
    let quit = use_context::<QuitCtx>();
    Portal::new()
        .is_open(quit.open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .on_close(move || quit.open.set(false))
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            vec![Box::new(card(use_context::<QuitCtx>().reasons.get()))]
        }))
}

/// «Выйти» — тот же путь, что и ✕ в шапке: [`WindowControl`] просит у
/// оконной системы закрытие, а перед этим снимает guard, чтобы второй
/// проход не открыл диалог снова.
fn confirm_button() -> impl Widget {
    WindowControl::close()
        .on_activate(|| {
            let quit = use_context::<QuitCtx>();
            quit.force.set(true);
            quit.open.set(false);
        })
        .child(
            Button::new(tr!("app.quit.confirm"))
                .leading_icon(MI_POWER_SETTINGS)
                .class("code-editor-dialog-btn-danger"),
        )
}

fn card(reasons: Vec<String>) -> impl Widget {
    let quit = use_context::<QuitCtx>();
    let cancel = move || quit.open.set(false);

    let list = Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(
            reasons
                .into_iter()
                .map(|r| {
                    Box::new(mgui! {
                        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                            Icon::new(MI_WARNING_AMBER).class("quit-dialog-bullet"),
                            Text::new(r).class("quit-dialog-reason"),
                        ]
                    }) as Box<dyn Widget>
                })
                .collect::<Vec<_>>(),
        );

    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card code-editor-dialog-danger quit-dialog") => [
            Column::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("app.quit.title")).class("code-editor-dialog-title"),
                    list,
                    Text::new(tr!("app.quit.hint")).class("code-editor-dialog-hint"),
                    Row::new()
                        .gap(10.0)
                        .main_axis_alignment(MainAxisAlignment::End) => [
                            Button::new(tr!("app.cancel"))
                                .leading_icon(MI_CLOSE)
                                .on_click(cancel)
                                .class("code-editor-dialog-btn-secondary"),
                            confirm_button(),
                        ],
                ]
        ]
    }
}
