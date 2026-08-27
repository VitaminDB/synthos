//! Подстраница «Архив» — чаты, убранные из рейла.
//!
//! Чат попадает сюда через «Закрыть» плитки в рейле или корзину в шапке
//! (`syn_chat::registry::archive`): файл остаётся на диске, история цела.
//! Отсюда его можно вернуть в рейл («Вернуть» — `registry::unarchive`,
//! чат сразу становится активным) или удалить насовсем (`registry::delete`
//! с подтверждением). «Очистить архив» удаляет всё разом — тоже через
//! подтверждение.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::dialog::{Dialog, DialogAction};

use crate::agent::state::ChatMeta;
use crate::components::chat_item::{display_title, initials_from_title, tone_for};
use crate::icons::*;
use crate::syn_chat::{registry, SynChatCtx};

pub fn view() -> impl Widget {
    // Диалоги подтверждения: удалить один чат (id) / очистить весь архив.
    let delete_target = use_signal(None::<ChatMeta>);
    let delete_open = use_signal(false);
    let clear_open = use_signal(false);

    let list = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<SynChatCtx>();
        let mut archived: Vec<ChatMeta> =
            ctx.chats.get().into_iter().filter(|m| m.archived).collect();
        archived.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        if archived.is_empty() {
            return vec![Box::new(empty_state())];
        }
        let mut col = Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch);
        for meta in archived {
            col = col.child(row(meta, delete_target, delete_open));
        }
        vec![Box::new(col)]
    });

    let header_row = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<SynChatCtx>();
        let count = ctx.chats.get().iter().filter(|m| m.archived).count();
        let clear = Button::new(tr!("settings.archive.clear"))
            .leading_icon(MI_DELETE_SWEEP)
            .on_click(move || clear_open.set(true))
            .disabled(count == 0)
            .class("settings-archive-clear");
        vec![Box::new(mgui! {
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                    Text::new(trn!("settings.archive.count", count)).class("settings-section-title"),
                    clear,
                ]
        })]
    });

    let delete_dialog = DecoratedBox::new().child(move || {
        let title = delete_target
            .get()
            .map(|m| display_title(&m.title))
            .unwrap_or_default();
        Dialog::new(tr!("settings.archive.delete_dialog.title", title = title))
            .body(tr!("settings.archive.delete_dialog.body"))
            .is_open(delete_open)
            .action(DialogAction::new(tr!("app.cancel"), move || delete_open.set(false)))
            .action(
                DialogAction::new(tr!("app.delete"), move || {
                    if let Some(meta) = delete_target.get_untracked() {
                        registry::delete(&meta.id);
                    }
                    delete_target.set(None);
                    delete_open.set(false);
                })
                .primary(),
            )
    });
    let clear_dialog = Dialog::new(tr!("settings.archive.clear_dialog.title"))
        .body(tr!("settings.archive.clear_dialog.body"))
        .is_open(clear_open)
        .action(DialogAction::new(tr!("app.cancel"), move || clear_open.set(false)))
        .action(
            DialogAction::new(tr!("settings.archive.clear"), move || {
                registry::clear_archive();
                clear_open.set(false);
            })
            .primary(),
        );

    mgui! {
        Stack::new().fit(StackFit::Expand) => [
            ScrollView::new().vertical() => [
                Padding::all(32.0) => [
                    DecoratedBox::new().class("settings-page") => [
                        Column::new().gap(24.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                            Text::new(tr!("settings.archive.title")).class("settings-page-title"),
                            Text::new(tr!("settings.archive.subtitle")).class("settings-page-subtitle"),
                            header_row,
                            list,
                        ]
                    ]
                ]
            ],
            delete_dialog,
            clear_dialog,
        ]
    }
}

fn empty_state() -> impl Widget {
    DecoratedBox::new().class("settings-card").child(Padding::all(24.0).child(mgui! {
        Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(MI_INBOX).class("settings-archive-empty-icon"),
            Text::new(tr!("settings.archive.empty.title")).class("settings-archive-empty-title"),
            Text::new(tr!("settings.archive.empty.hint")).class("settings-archive-empty-hint"),
        ]
    }))
}

/// Строка архива: аватар, название, превью и дата; справа — «Вернуть» и
/// «Удалить».
fn row(
    meta: ChatMeta,
    delete_target: RwSignal<Option<ChatMeta>>,
    delete_open: RwSignal<bool>,
) -> impl Widget {
    let id_restore = meta.id.clone();
    let meta_delete = meta.clone();
    let avatar = Avatar::new()
        .text(initials_from_title(&meta.title))
        .size(36.0)
        .class(tone_for(&meta.id));
    let preview = if meta.preview.trim().is_empty() {
        tr!("chat.item.no_messages")
    } else {
        meta.preview.clone()
    };
    let date = format_date(meta.updated_at);
    let model = meta.model_name.clone().unwrap_or_default();
    let subtitle = if model.is_empty() {
        date
    } else {
        format!("{date} · {model}")
    };

    DecoratedBox::new().class("settings-card settings-archive-row").child(mgui! {
        Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            avatar,
            DecoratedBox::new().class("grow").child(mgui! {
                Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                    Text::new(display_title(&meta.title)).max_lines(1).class("settings-archive-title"),
                    Text::new(preview).max_lines(1).class("settings-archive-preview"),
                    Text::new(subtitle).max_lines(1).class("settings-archive-meta"),
                ]
            }),
            Button::new(tr!("settings.archive.restore"))
                .leading_icon(MI_UNARCHIVE)
                .on_click(move || {
                    registry::unarchive(&id_restore);
                    crate::rail::navigate("syn_chat");
                })
                .class("settings-archive-restore"),
            ToolButton::new(MI_DELETE_FOREVER)
                .tooltip(tr!("settings.archive.delete"))
                .on_click(move || {
                    delete_target.set(Some(meta_delete.clone()));
                    delete_open.set(true);
                })
                .class("settings-archive-delete"),
        ]
    })
}

/// `YYYY-MM-DD` из unix-секунд — без зависимостей от chrono.
fn format_date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    // Алгоритм civil_from_days (Howard Hinnant).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}
