//! Плавающее окно редактора системного промпта и модальные диалоги
//! библиотеки пресетов (создать / переименовать / удалить / заменить текст
//! чата текстом пресета).
//!
//! Оба смонтированы в корне страницы чата (`pages::syn_chat::view`), а не в
//! карточке правой панели: карточка пересобирается на каждое нажатие
//! клавиши и живёт только на табе «Параметры», а окно должно переживать
//! переключение табов. Состояние — в `SynChatCtx` (`prompt_window_*`,
//! `prompt_dialog`), поэтому пересборка страницы его не теряет.
//!
//! Окно и редактор в карточке правят один сигнал `system_prompt`: набранное
//! в окне сразу видно в панели и наоборот.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::{Checkbox, FloatingWindow, MultilineTextEdit, TextField};

use crate::icons::{MI_ADD, MI_CHECK, MI_CLOSE, MI_DELETE, MI_DESCRIPTION};
use crate::syn_chat::prompt_presets::{self, PromptDialog};
use crate::syn_chat::SynChatCtx;

// ─────────────────────────── окно ───────────────────────────

/// Плавающее окно с многострочным редактором промпта открытого чата на всю
/// площадь. Заголовок — имя пресета, из которого взят текст; пересобирается
/// только при смене пресета или его имени, текст редактора живёт во
/// вложенной реактивной ветке.
pub fn window() -> impl Widget {
    DecoratedBox::new().child(|| {
        let ctx = use_context::<SynChatCtx>();
        let active = ctx.prompt_active.get();
        let name = ctx
            .prompt_presets
            .get()
            .into_iter()
            .find(|p| p.id == active)
            .map(|p| p.name)
            .unwrap_or_default();
        let title = if name.trim().is_empty() {
            tr!("chat.right.system.window.title")
        } else {
            tr!("chat.right.system.window.title_named", name = name)
        };
        FloatingWindow::new(title)
            .icon(MI_DESCRIPTION)
            .is_open(ctx.prompt_window_open)
            .position(ctx.prompt_window_pos)
            .size(ctx.prompt_window_size.get_untracked())
            .size_signal(ctx.prompt_window_size)
            .centered()
            .with_resizable(true)
            .closable(true)
            .child(window_body())
            .class("system-prompt-window")
    })
}

fn window_body() -> impl Widget {
    DecoratedBox::new().child(|| {
        let ctx = use_context::<SynChatCtx>();
        let text = ctx.system_prompt.get();
        let chars = text.chars().count();
        let lines = if text.is_empty() { 0 } else { text.lines().count() };

        let editor = MultilineTextEdit::new()
            .text(text)
            .placeholder(tr!("chat.right.system.placeholder"))
            .rows(12)
            .soft_wrap(true)
            .on_change(|s| {
                use_context::<SynChatCtx>().system_prompt.set(s.to_string());
            })
            .class("system-prompt-window-edit");
        let stats = Text::new(tr!("chat.right.system.window.stats", chars = chars, lines = lines))
            .class("system-prompt-window-stats");

        mgui! {
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    editor,
                    stats,
                ]
        }
    })
}

// ─────────────────────────── диалоги ───────────────────────────

/// Portal с карточкой создания / переименования / удаления пресета.
/// Источник — `SynChatCtx.prompt_dialog`.
pub fn dialog() -> impl Widget {
    let is_open = use_signal(false);
    create_effect(move || {
        let ctx = use_context::<SynChatCtx>();
        let want = ctx.prompt_dialog.get().is_some();
        if is_open.get_untracked() != want {
            is_open.set(want);
        }
    });

    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .on_close(close_dialog)
        .child(Reactive::new(|| -> Vec<Box<dyn Widget>> {
            let ctx = use_context::<SynChatCtx>();
            let Some(kind) = ctx.prompt_dialog.get() else {
                return vec![Box::new(DecoratedBox::new().class("code-editor-dialog-empty"))];
            };
            let card: Box<dyn Widget> = match kind {
                PromptDialog::Create => Box::new(create_card()),
                PromptDialog::Rename { id, name } => Box::new(rename_card(id, name)),
                PromptDialog::Delete { id, name } => Box::new(delete_card(id, name)),
                PromptDialog::Replace { id, name } => Box::new(replace_card(id, name)),
            };
            vec![card]
        }))
}

fn close_dialog() {
    use_context::<SynChatCtx>().prompt_dialog.set(None);
}

/// Поле имени. `on_submit` не используем: в syngui он стреляет и на потерю
/// фокуса, так что клик по чекбоксу или «Отмена» превращался бы в
/// подтверждение (см. `pages::settings::skills_dialog`).
fn name_field(name: RwSignal<String>) -> impl Widget {
    TextField::new()
        .text(name.get_untracked())
        .placeholder(tr!("chat.right.system.dialog.name.placeholder"))
        .autofocus(true)
        .on_change(move |s| name.set(s.to_string()))
        .on_escape(close_dialog)
        .class("code-editor-dialog-input")
}

fn create_card() -> impl Widget {
    let ctx = use_context::<SynChatCtx>();
    let name = use_signal(prompt_presets::suggest_name(&ctx));
    // Промпт принадлежит чату: без копии чат получил бы пустой текст
    // нового пресета, поэтому по умолчанию пресет берёт текст чата.
    let copy_current = use_signal(true);

    let confirm = move || {
        let ctx = use_context::<SynChatCtx>();
        let n = name.get_untracked();
        if n.trim().is_empty() {
            return;
        }
        prompt_presets::create(&ctx, &n, copy_current.get_untracked());
        ctx.prompt_dialog.set(None);
    };

    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card") => [
            Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(tr!("chat.right.system.dialog.create.title")).class("code-editor-dialog-title"),
                Text::new(tr!("chat.right.system.dialog.create.hint")).class("code-editor-dialog-hint"),
                name_field(name),
                Checkbox::checked(true)
                    .label(tr!("chat.right.system.dialog.create.copy_current"))
                    .on_change(move |v| copy_current.set(v)),
                Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::End) => [
                    Button::new(tr!("app.cancel"))
                        .leading_icon(MI_CLOSE)
                        .on_click(close_dialog)
                        .class("code-editor-dialog-btn-secondary"),
                    Button::new(tr!("chat.right.system.dialog.create.confirm"))
                        .leading_icon(MI_ADD)
                        .on_click(confirm)
                        .class("code-editor-dialog-btn-primary"),
                ],
            ]
        ]
    }
}

fn rename_card(id: String, current: String) -> impl Widget {
    let name = use_signal(current);

    let confirm = move || {
        let ctx = use_context::<SynChatCtx>();
        let n = name.get_untracked();
        if n.trim().is_empty() {
            return;
        }
        prompt_presets::rename(&ctx, &id, &n);
        ctx.prompt_dialog.set(None);
    };

    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card") => [
            Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(tr!("chat.right.system.dialog.rename.title")).class("code-editor-dialog-title"),
                name_field(name),
                Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::End) => [
                    Button::new(tr!("app.cancel"))
                        .leading_icon(MI_CLOSE)
                        .on_click(close_dialog)
                        .class("code-editor-dialog-btn-secondary"),
                    Button::new(tr!("chat.right.system.dialog.rename.confirm"))
                        .leading_icon(MI_CHECK)
                        .on_click(confirm)
                        .class("code-editor-dialog-btn-primary"),
                ],
            ]
        ]
    }
}

fn delete_card(id: String, name: String) -> impl Widget {
    let confirm = move || {
        let ctx = use_context::<SynChatCtx>();
        prompt_presets::delete(&ctx, &id);
        ctx.prompt_dialog.set(None);
    };

    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card code-editor-dialog-danger") => [
            Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(tr!("chat.right.system.dialog.delete.title", name = name))
                    .class("code-editor-dialog-title"),
                Text::new(tr!("chat.right.system.dialog.delete.hint")).class("code-editor-dialog-hint"),
                Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::End) => [
                    Button::new(tr!("app.cancel"))
                        .leading_icon(MI_CLOSE)
                        .on_click(close_dialog)
                        .class("code-editor-dialog-btn-secondary"),
                    Button::new(tr!("app.delete"))
                        .leading_icon(MI_DELETE)
                        .on_click(confirm)
                        .class("code-editor-dialog-btn-primary"),
                ],
            ]
        ]
    }
}

/// Выбор другого пресета, когда текст чата изменён: подтвердить, что правка
/// пропадёт (`prompt_presets::request_select`).
fn replace_card(id: String, name: String) -> impl Widget {
    let confirm = move || {
        let ctx = use_context::<SynChatCtx>();
        prompt_presets::select(&ctx, &id);
        ctx.prompt_dialog.set(None);
    };

    mgui! {
        DecoratedBox::new().class("code-editor-dialog-card") => [
            Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(tr!("chat.right.system.dialog.replace.title", name = name))
                    .class("code-editor-dialog-title"),
                Text::new(tr!("chat.right.system.dialog.replace.hint")).class("code-editor-dialog-hint"),
                Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::End) => [
                    Button::new(tr!("app.cancel"))
                        .leading_icon(MI_CLOSE)
                        .on_click(close_dialog)
                        .class("code-editor-dialog-btn-secondary"),
                    Button::new(tr!("chat.right.system.dialog.replace.confirm"))
                        .leading_icon(MI_CHECK)
                        .on_click(confirm)
                        .class("code-editor-dialog-btn-primary"),
                ],
            ]
        ]
    }
}
