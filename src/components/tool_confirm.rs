//! Диалог подтверждения исполнения tool-call’а.
//!
//! Живёт в overlay-слое через [`Portal`], монтируется один раз в
//! `pages/chat.rs`. Открытие/закрытие — реактивно через
//! `AppCtx.tools.pending_approval`; оркестратор в `chat::session`
//! заблокирован в `await`, пока пользователь не нажмёт одну из трёх
//! кнопок — канал `oneshot`, спрятанный в `PendingApproval.sender`,
//! доставляет решение обратно.
//!
//! Три кнопки:
//! - «Отмена» — прервать текущий tool-turn (записывается tool-result
//!   с `error=true, content="Отменено пользователем"`).
//! - «Разрешить» — выполнить только этот вызов.
//! - «Разрешить все» — добавить чат в `tools.allow_all_chats`, выполнить этот
//!   и все последующие вызовы до конца чата без диалога.

use std::sync::Arc;

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::overlay::PortalAnchor;
use syngui::StyledWidget;

use crate::agent::tools::{PendingApproval, Tool, ToolDecision};
use crate::context::AppCtx;
use crate::icons::{MI_CHECK, MI_CLOSE, MI_DONE_ALL};

pub fn view() -> impl Widget {
    // is_open — derived-сигнал от pending_approval.is_some(). Portal
    // требует RwSignal<bool>, поэтому держим отдельный и синхронизируем
    // через create_effect.
    let is_open = use_signal(false);
    create_effect(move || {
        let ctx = use_context::<AppCtx>();
        let has_pending = ctx.tools.pending_approval.get().is_some();
        if is_open.get_untracked() != has_pending {
            is_open.set(has_pending);
        }
    });

    Portal::new()
        .is_open(is_open)
        .modal(true)
        .backdrop(true)
        .anchor(PortalAnchor::Center)
        .child(card())
}

/// Карточка диалога — реактивный child Portal’а. Если pending = None,
/// отрендерим пустой DecoratedBox (визуально не видно из-за закрытого
/// Portal).
fn card() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let Some(pending) = ctx.tools.pending_approval.get() else {
            return DecoratedBox::new().class("tool-confirm-empty");
        };

        card_inner(pending)
    }
}

fn card_inner(pending: Arc<PendingApproval>) -> StyledWidget<DecoratedBox> {
    let tool_icon = pending.tool_icon.clone();
    let tool_label = Tool::by_key(&pending.tool_key)
        .map(crate::i18n::tool_label)
        .unwrap_or_else(|| pending.tool_label.clone());
    let args_pretty = if pending.args_pretty.trim().is_empty() {
        tr!("chat.tool_confirm.no_args")
    } else {
        pending.args_pretty.clone()
    };

    let header = mgui! {
        Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            DecoratedBox::new().class("tool-confirm-icon-wrap") => [
                Center::new() => [ Icon::new(tool_icon).class("tool-confirm-icon") ]
            ],
            DecoratedBox::new().class("grow") => [
                Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                    Text::new(tr!("chat.tool_confirm.title")).class("tool-confirm-title"),
                    Text::new(tr!("chat.tool_confirm.tool", name = tool_label)).class("tool-confirm-subtitle"),
                ]
            ],
        ]
    };

    // Длинные tool-call'ы (HTML/JSON в 5–10 KB) раньше распирали карточку
    // на весь экран и кнопки уезжали за нижний край. Решение:
    //   – `.tool-confirm-card` ограничен `max-height: 80%` viewport;
    //   – args_block обёрнут в `.grow` (flex-grow: 1) внутри карточки —
    //     забирает всё свободное место между header и warning/buttons;
    //   – содержимое (Text с `white-space: pre`) лежит в ScrollView::both(),
    //     чтобы и вертикаль, и длинные строки кода скроллились.
    let args_block = DecoratedBox::new().class("tool-confirm-args-wrap").child(
        Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new(tr!("chat.tool_confirm.args")).class("tool-confirm-args-label"))
            .child(
                ScrollView::new()
                    .both()
                    .class("tool-confirm-args-scroll")
                    .child(Text::new(args_pretty).class("tool-confirm-args")),
            ),
    );

    let pending_cancel = pending.clone();
    let pending_allow = pending.clone();
    let pending_allow_all = pending.clone();

    let buttons = mgui! {
        Row::new().gap(10.0).main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Button::new(tr!("app.cancel"))
                .leading_icon(MI_CLOSE)
                .on_click(move || decide(&pending_cancel, ToolDecision::Cancel))
                .class("tool-confirm-btn tool-confirm-btn-secondary"),
            Button::new(tr!("chat.tool_confirm.allow"))
                .leading_icon(MI_CHECK)
                .on_click(move || decide(&pending_allow, ToolDecision::Allow))
                .class("tool-confirm-btn tool-confirm-btn-primary"),
            Button::new(tr!("chat.tool_confirm.allow_all"))
                .leading_icon(MI_DONE_ALL)
                .on_click(move || decide(&pending_allow_all, ToolDecision::AllowAll))
                .class("tool-confirm-btn tool-confirm-btn-accent"),
        ]
    };

    // Карточка растёт по контенту: маленькие аргументы → маленькое окно;
    // длинные → ScrollView внутри args_block берёт max-height из MSS, а
    // сама карточка ограничивается `max-height: 80%` от viewport (см. MSS).
    // Никакого `.grow` — иначе блок растягивался бы на всю высоту даже
    // при двух строках JSON.
    DecoratedBox::new().class("tool-confirm-card").child(mgui! {
        Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            header,
            args_block,
            Text::new(tr!("chat.tool_confirm.warning")).class("tool-confirm-warning"),
            buttons,
        ]
    })
}

/// Универсальный обработчик решения: отправляет результат в oneshot-канал
/// и мгновенно убирает pending из сигнала, чтобы Portal схлопнулся до того,
/// как оркестратор продолжит. Оркестратор после пробуждения тоже вызывает
/// `pending_approval.set(None)` — двойной `set` с тем же значением `None`
/// безопасен (PartialEq-шорткат).
fn decide(pending: &Arc<PendingApproval>, decision: ToolDecision) {
    pending.send(decision);
    use_context::<AppCtx>().tools.pending_approval.set(None);
}
