//! Маршрут «Поддержка» — живой stdout/stderr `llama-server`.
//!
//! Читает `AppCtx.llama.logs_signal` как источник строк и рендерит их в
//! виртуализированный ListView. Статус и адрес сервера — реактивный
//! header-бар. Кнопка «Очистить лог» опустошает буфер.

use syngui::mgui;
use syngui::prelude::*;

use crate::context::AppCtx;
use crate::icons::*;
use crate::llama::ProcessStatus;

pub fn view() -> impl Widget {
    mgui! {
        DecoratedBox::new().class("support-shell") => [
            Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                header(),
                DecoratedBox::new().class("support-log-wrap grow").child(log_body()),
            ]
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Верхняя панель: иконка, статус-пилюля, host:port, кнопка очистки
// ─────────────────────────────────────────────────────────────────────────────

fn header() -> impl Widget {
    DecoratedBox::new().class("support-header").child(
        Padding::symmetric(24.0, 16.0).child(mgui! {
            Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().class("support-header-icon-wrap") => [
                    Center::new().child(Icon::new(MI_TERMINAL).class("support-header-icon")),
                ],
                DecoratedBox::new().class("grow") => [
                    Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                        Text::new("Поддержка · llama-server").class("support-header-title"),
                        header_endpoint()
                    ]
                ],
                status_pill(),
                clear_button(),
            ]
        }),
    )
}

fn header_endpoint() -> impl Widget {
    // Реактивно показываем настроенный host:port и путь к бинарю.
    DecoratedBox::new().class("support-endpoint").child(move || {
        let ctx = use_context::<AppCtx>();
        let host = ctx.general.server_host.get();
        let port = ctx.general.server_port.get();
        let path = ctx.general.server_path.get();
        let text = format!("{host}:{port}  ·  {}", if path.trim().is_empty() { "llama-server" } else { path.as_str() });
        Text::new(text).class("support-endpoint-text")
    })
}

fn status_pill() -> impl Widget {
    // Фиксированная обёртка даёт Row intrinsic-size — Reactive внутри
    // без неё меряется в 0 (см. компонент llama_control для контекста).
    DecoratedBox::new().class("support-pill-slot").child(status_pill_reactive())
}

fn status_pill_reactive() -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let s = ctx.llama.status_signal.get();
        let class = format!("llama-pill llama-pill-{}", s.css_modifier());
        DecoratedBox::new().class(class.as_str()).child(
            Center::new().child(Text::new(s.label()).class("llama-pill-text")),
        )
    }
}

fn clear_button() -> impl Widget {
    Button::new("Очистить")
        .leading_icon(MI_CLEAR_ALL)
        .on_click(|| {
            let ctx = use_context::<AppCtx>();
            ctx.llama.clear_logs();
        })
        .class("support-clear-btn")
}

// ─────────────────────────────────────────────────────────────────────────────
// Тело: виртуализированный список строк лога
// ─────────────────────────────────────────────────────────────────────────────

fn log_body() -> impl Widget {
    DecoratedBox::new().class("support-log-area").child(move || {
        let ctx = use_context::<AppCtx>();
        let logs = ctx.llama.logs_signal.get();
        let status = ctx.llama.status_signal.get();

        let child: Box<dyn Widget> = if logs.is_empty() {
            Box::new(empty_state(status))
        } else {
            Box::new(log_list(logs))
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    })
}

fn empty_state(status: ProcessStatus) -> impl Widget {
    let hint = match status {
        ProcessStatus::Running | ProcessStatus::Starting => {
            "Сервер запущен — ждём первых строк вывода…"
        }
        _ => "Нажмите «Запустить» в панели «Ламма» справа, чтобы увидеть логи сервера.",
    };
    mgui! {
        Center::new() => [
            Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().class("support-empty-bubble") => [
                    Center::new().child(Icon::new(MI_TERMINAL).class("support-empty-icon")),
                ],
                Text::new("Консоль пуста").class("support-empty-title"),
                Padding::symmetric(32.0, 0.0).child(
                    Text::new(hint).class("support-empty-subtitle"),
                ),
            ]
        ]
    }
}

fn log_list(logs: Vec<String>) -> impl Widget {
    // ListView с custom-item_widget: моноширинная строка текста.
    // Eager режим допустим: cap 5000 строк (см. process::MAX_LOG_LINES).
    let items: Vec<ListItem> = logs.into_iter().map(ListItem::new).collect();
    ListView::new(items)
        .item_height(18.0)
        .item_widget(|_idx, item, _sel, _hov| {
            Box::new(
                Padding::symmetric(16.0, 0.0).child(
                    Text::new(item.text.clone()).class("support-log-line"),
                ),
            )
        })
        .class("support-log-list")
}
