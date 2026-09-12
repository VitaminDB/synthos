//! Панель визарда в ленте: вопрос ассистента с кнопками-вариантами и/или
//! полем свободного ответа (инструмент `wizard`, см.
//! `agent::tools::wizard`). Клик по варианту (или «Отправить» при
//! множественном выборе) уходит обычным сообщением пользователя через
//! `session::send_message`; после ответа панель сворачивается в строку с
//! вопросом — сам ответ виден следующим пузырьком.
//!
//! Состояние (выбор, текст, таймер) живёт в `SynChatCtx::wizard_drafts` по
//! индексу сообщения: сигнал внутри ленты пересоздавался бы при каждом её
//! ребилде. Панель — своя `Reactive`, подписанная на черновики и секундный
//! тик таймера, поэтому обратный отсчёт не перестраивает всю ленту.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use syngui::async_runtime::run_on_main_thread;
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::visual::markdown_view::linearize_markdown_source;
use syngui::widgets::{Chip, Flex, ProgressBar, Reactive, Tooltip};

use super::message_bubble::bubble_markdown;
use crate::agent::tools::wizard::{answer_text, parse_spec, WizardSpec};
use crate::icons::{
    MI_CHECK, MI_CHECK_BOX, MI_CHECK_BOX_OUTLINE_BLANK, MI_HELP_OUTLINE, MI_SEND, MI_TIMER,
};
use crate::syn_chat::session;
use crate::syn_chat::state::{ChatMsg, SynChatCtx, WizardDraft};

/// Спецификация панели из tool_call-сообщения: аргументы вызова, иначе
/// тело (там тот же JSON, отформатированный).
pub fn spec_of(msg: &ChatMsg) -> Option<WizardSpec> {
    let raw = msg
        .tool_calls
        .as_ref()
        .and_then(|c| c.first())
        .and_then(|c| c.function.arguments.clone())
        .unwrap_or_else(|| msg.body.clone());
    parse_spec(&raw).ok()
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Строка ленты: аватар ассистента + подпись + панель.
pub fn row(msg: &ChatMsg, msg_idx: usize, spec: WizardSpec) -> impl Widget {
    let ctx = use_context::<SynChatCtx>();
    // После результата вызова (idx+1) в ленте есть ещё что-то — значит,
    // пользователь ответил (или разговор ушёл дальше).
    let answered = ctx.messages.with_untracked(Vec::len) > msg_idx + 2;
    if !answered {
        arm_timer(&ctx, msg_idx, &spec);
    }

    let avatar = Avatar::new()
        .text(msg.initials.clone())
        .size(32.0)
        .class(msg.tone_class.clone());
    let author_row = mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new(msg.author.clone()).class("msg-author"),
            Text::new(tr!("chat.msg.wizard.hint")).class("tool-call-hint"),
            Text::new(msg.time.clone()).class("msg-time"),
        ]
    };
    let panel = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<SynChatCtx>();
        let draft = ctx.wizard_drafts.get().get(&msg_idx).cloned().unwrap_or_default();
        let _ = ctx.wizard_tick.get();
        let status = if answered || draft.dismissed {
            Some(if draft.dismissed && !answered {
                tr!("chat.msg.wizard.skipped")
            } else {
                tr!("chat.msg.wizard.answered")
            })
        } else if draft.expired {
            Some(tr!("chat.msg.wizard.expired"))
        } else {
            None
        };
        vec![match status {
            Some(status) => Box::new(collapsed(&spec, status)),
            None => Box::new(live(msg_idx, &spec, &draft)),
        }]
    });
    let meta = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(vec![Box::new(author_row) as Box<dyn Widget>, Box::new(panel)]);
    mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Start).main_axis_alignment(MainAxisAlignment::Start) => [
            avatar,
            meta,
        ]
    }
}

/// Таймер: срок ставится один раз при первом показе живой панели, тикер
/// раз в секунду бампает `wizard_tick` и по сроку сворачивает панель.
/// Черновики сбрасываются при смене чата — вернувшись, пользователь
/// получает отсчёт заново; тикер чужого чата панель не трогает.
fn arm_timer(ctx: &SynChatCtx, msg_idx: usize, spec: &WizardSpec) {
    let Some(secs) = spec.timeout_sec else { return };
    if ctx.wizard_drafts.get_untracked().get(&msg_idx).and_then(|d| d.deadline).is_some() {
        return;
    }
    let chat_id = ctx.active_chat_id.get_untracked();
    // Запись в сигнал не из сборки ленты, а следующим тиком.
    run_on_main_thread(move || {
        let ctx = use_context::<SynChatCtx>();
        let already = ctx.wizard_drafts.get_untracked().get(&msg_idx).and_then(|d| d.deadline).is_some();
        if already || ctx.active_chat_id.get_untracked() != chat_id {
            return;
        }
        let deadline = unix_now() + secs;
        ctx.wizard_drafts.update(|m| m.entry(msg_idx).or_default().deadline = Some(deadline));
        syngui::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let done = unix_now() >= deadline;
                let chat_id = chat_id.clone();
                run_on_main_thread(move || {
                    let ctx = use_context::<SynChatCtx>();
                    if ctx.active_chat_id.get_untracked() != chat_id {
                        return;
                    }
                    let still_mine = ctx
                        .wizard_drafts
                        .get_untracked()
                        .get(&msg_idx)
                        .map(|d| d.deadline == Some(deadline))
                        .unwrap_or(false);
                    if !still_mine {
                        return;
                    }
                    ctx.wizard_tick.update(|v| *v += 1);
                    if done {
                        ctx.wizard_drafts.update(|m| {
                            if let Some(d) = m.get_mut(&msg_idx) {
                                d.expired = true;
                            }
                        });
                    }
                });
                if done {
                    break;
                }
            }
        });
    });
}

/// Свёрнутая панель: вопрос одной строкой и пометка (отвечено /
/// пропущено / время вышло).
fn collapsed(spec: &WizardSpec, status: String) -> impl Widget {
    mgui! {
        DecoratedBox::new().class("wizard-card wizard-card-collapsed") => [
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_HELP_OUTLINE).class("wizard-icon"),
                Text::new(question_summary(&spec.question)).class("wizard-question-collapsed"),
                Text::new(status).class("wizard-status"),
            ]
        ]
    }
}

/// Вопрос одной строкой для свёрнутой панели: разметка снята, берётся
/// первая непустая строка, длинная обрезается с многоточием.
pub fn question_summary(question: &str) -> String {
    const MAX_CHARS: usize = 96;
    let plain = linearize_markdown_source(question);
    let line = plain
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() > MAX_CHARS {
        let cut: String = line.chars().take(MAX_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        line.to_string()
    }
}

/// Правка черновика панели; контекст берётся на месте — вызывается из
/// обработчиков кликов, которым захватывать `SynChatCtx` неудобно.
fn update_draft(msg_idx: usize, f: impl FnOnce(&mut WizardDraft)) {
    use_context::<SynChatCtx>()
        .wizard_drafts
        .update(|m| f(m.entry(msg_idx).or_default()));
}

/// Отправить ответ обычным сообщением и свернуть панель.
fn submit(msg_idx: usize, spec: &WizardSpec, draft: &WizardDraft) {
    let text = answer_text(spec, &draft.selected, &draft.custom);
    if text.trim().is_empty() {
        return;
    }
    update_draft(msg_idx, |d| d.dismissed = true);
    session::send_message(text);
}

/// Кнопка/чип с подсказкой, если она задана.
fn with_tip<W: Widget + 'static>(w: W, tip: &str) -> Box<dyn Widget> {
    if tip.is_empty() {
        Box::new(w)
    } else {
        Box::new(Tooltip::new(w, tip.to_string()))
    }
}

/// Подписи длиннее этого — варианты столбиком на всю ширину карточки,
/// иначе — в строку с переносом (короткие ответы читаются как чипы).
const WIDE_LABEL_CHARS: usize = 36;

/// Последние секунды отсчёта подсвечиваются.
const URGENT_SECS: u64 = 10;

/// Живая панель: вопрос, варианты, поле, подвал с таймером и кнопками.
fn live(msg_idx: usize, spec: &WizardSpec, draft: &WizardDraft) -> impl Widget {
    let mut items: Vec<Box<dyn Widget>> = Vec::new();

    // Шапка: значок в кружке и вопрос той же MarkdownView, что у ответов
    // ассистента — код с подсветкой, списки, жирный, а не сырой текст.
    items.push(Box::new(mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
            DecoratedBox::new().class("wizard-icon-badge") => [
                Padding::all(5.0) => [
                    Icon::new(MI_HELP_OUTLINE).class("wizard-icon"),
                ],
            ],
            DecoratedBox::new().class("grow wizard-question") => [
                bubble_markdown(&spec.question, "msg-bubble-md wizard-question-md"),
            ],
        ]
    }));

    // Варианты: одиночный выбор — кнопки, клик отправляет сразу;
    // множественный — чипы-флажки и «Отправить». Короткие подписи — в
    // строку с переносом, длинные — столбиком на всю ширину.
    let stacked = !spec.allow_multiple
        && spec.options.iter().any(|o| o.label.chars().count() > WIDE_LABEL_CHARS);
    let mut options: Vec<Box<dyn Widget>> = Vec::new();
    for (i, o) in spec.options.iter().enumerate() {
        let selected = draft.selected.contains(&i);
        let spec_c = spec.clone();
        let draft_c = draft.clone();
        let option_class = match (stacked, selected) {
            (true, true) => "wizard-option wizard-option-wide selected",
            (true, false) => "wizard-option wizard-option-wide",
            (false, true) => "wizard-option selected",
            (false, false) => "wizard-option",
        };
        let option: Box<dyn Widget> = if spec.allow_multiple {
            with_tip(
                Chip::new(o.label.clone())
                    .icon(if selected { MI_CHECK_BOX } else { MI_CHECK_BOX_OUTLINE_BLANK })
                    .selected(selected)
                    .on_click(move || {
                        update_draft(msg_idx, |d| {
                            if let Some(p) = d.selected.iter().position(|&x| x == i) {
                                d.selected.remove(p);
                            } else {
                                d.selected.push(i);
                            }
                        })
                    })
                    .class(if selected { "wizard-chip selected" } else { "wizard-chip" }),
                &o.tooltip,
            )
        } else if o.allow_free_text {
            with_tip(
                Button::new(o.label.clone())
                    .on_click(move || {
                        update_draft(msg_idx, |d| {
                            d.selected = vec![i];
                            d.custom_open = true;
                        })
                    })
                    .class(option_class),
                &o.tooltip,
            )
        } else {
            with_tip(
                Button::new(o.label.clone())
                    .on_click(move || {
                        let mut d = draft_c.clone();
                        d.selected = vec![i];
                        submit(msg_idx, &spec_c, &d);
                    })
                    .class(option_class),
                &o.tooltip,
            )
        };
        options.push(option);
    }
    if !options.is_empty() {
        let list: Box<dyn Widget> = if stacked {
            Box::new(
                Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(options),
            )
        } else {
            Box::new(
                Flex::row()
                    .wrap()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(options),
            )
        };
        items.push(list);
    }

    // Поле свободного ответа: общее — всегда; у варианта «свой» — после
    // его выбора. Enter отправляет.
    let custom_visible = spec.allow_free_text || draft.custom_open || spec.options.is_empty();
    if custom_visible {
        let spec_s = spec.clone();
        let draft_s = draft.clone();
        items.push(Box::new(
            TextField::new()
                .text(draft.custom.clone())
                .placeholder(tr!("chat.msg.wizard.custom_placeholder"))
                .autofocus(draft.custom_open)
                .on_change(move |s: &str| {
                    let s = s.to_string();
                    update_draft(msg_idx, |d| d.custom = s);
                })
                .on_submit(move |s: &str| {
                    let mut d = draft_s.clone();
                    d.custom = s.to_string();
                    submit(msg_idx, &spec_s, &d);
                })
                .class("wizard-custom"),
        ));
    }

    // Подвал под тонкой линией: полоса остатка времени · таймер ·
    // «Нужен ответ» · «Пропустить» (если не обязателен) · «Отправить».
    let mut footer_rows: Vec<Box<dyn Widget>> = Vec::new();
    let mut footer = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(deadline) = draft.deadline {
        let left = deadline.saturating_sub(unix_now());
        let total = spec.timeout_sec.unwrap_or(0).max(1);
        footer_rows.push(Box::new(
            ProgressBar::with_value(left as f32 / total as f32).class("wizard-timer-bar"),
        ));
        let urgent = left <= URGENT_SECS;
        footer = footer.child(
            Icon::new(MI_TIMER).class(if urgent { "wizard-footer-icon urgent" } else { "wizard-footer-icon" }),
        );
        footer = footer.child(
            Text::new(tr!("chat.msg.wizard.timer", n = left))
                .class(if urgent { "wizard-footer-text urgent" } else { "wizard-footer-text" }),
        );
    }
    if spec.required {
        footer = footer.child(Text::new(tr!("chat.msg.wizard.required")).class("wizard-footer-text"));
    }
    footer = footer.child(DecoratedBox::new().class("grow"));
    if !spec.required {
        footer = footer.child(
            Button::new(tr!("chat.msg.wizard.skip"))
                .on_click(move || update_draft(msg_idx, |d| d.dismissed = true))
                .class("code-editor-dialog-btn-secondary"),
        );
    }
    let needs_send_button = spec.allow_multiple || custom_visible;
    if needs_send_button {
        let ready = !answer_text(spec, &draft.selected, &draft.custom).trim().is_empty();
        let spec_b = spec.clone();
        let draft_b = draft.clone();
        footer = footer.child(
            Button::new(tr!("chat.msg.wizard.send"))
                .leading_icon(if spec.allow_multiple { MI_CHECK } else { MI_SEND })
                .disabled(!ready)
                .on_click(move || submit(msg_idx, &spec_b, &draft_b))
                .class("code-editor-dialog-btn-primary"),
        );
    }
    footer_rows.push(Box::new(footer));
    items.push(Box::new(DecoratedBox::new().class("wizard-footer").child(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(footer_rows),
    )));

    DecoratedBox::new().class("wizard-card").child(
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(items),
    )
}

#[cfg(test)]
mod tests {
    use super::question_summary;

    #[test]
    fn summary_takes_first_plain_line_and_truncates() {
        assert_eq!(question_summary("**Какой** период?"), "Какой период?");
        assert_eq!(
            question_summary("Что делает код?\n\n```rust\nfn main() {}\n```"),
            "Что делает код?"
        );
        assert_eq!(question_summary("\n\n  \n# Заголовок\nтекст"), "Заголовок");
        let long = "а".repeat(200);
        let s = question_summary(&long);
        assert_eq!(s.chars().count(), 96);
        assert!(s.ends_with('…'));
        assert_eq!(question_summary(""), "");
    }
}
