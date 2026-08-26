//! Подстраница «Скилы» — main-колонка редактора.
//!
//! Реактивно следит за `skills_selected_id`: при `None` показывает
//! приглашение выбрать или создать; при выборе — отрисовывает заголовок
//! и `CodeEditor` с подсветкой Markdown. Изменения текста уходят в
//! `AppCtx.skills` (in-memory) и асинхронно на диск через `crate::skills::save`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::input::code_editor::syntax::Language;
use syngui::widgets::CodeEditor;

use crate::context::{AppCtx, SkillDialogKind};
use crate::icons::*;
use crate::skills;

/// Дебаунс автосохранения скила в `~/.config/synthos/skills/<id>.md` (мс).
const AUTOSAVE_DEBOUNCE_MS: u64 = 400;

pub fn view() -> impl Widget {
    DecoratedBox::new().class("settings-page skills-page").child(move || {
        let ctx = use_context::<AppCtx>();
        // Подписка на skills и selected_id — Reactive пересоберёт editor
        // при смене выделения или после CRUD.
        let skills = ctx.skills.get();
        let selected = ctx.skills_selected_id.get();
        let child: Box<dyn Widget> = match selected.as_ref().and_then(|id| {
            skills.iter().find(|s| &s.id == id).cloned()
        }) {
            Some(skill) => Box::new(editor(skill)),
            None => Box::new(empty_state(skills.len())),
        };
        Stack::new().fit(StackFit::Expand).children(vec![child])
    })
}

fn editor(skill: crate::skills::Skill) -> impl Widget {
    let id_for_header = skill.id.clone();
    let name = skill.name.clone();
    let description = skill.description.clone();
    let description_for_edit = description.clone();
    let initial = skill.content.clone();
    let id_for_change = skill.id.clone();

    mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            DecoratedBox::new().class("skill-editor-header") => [
                Padding::symmetric(24.0, 16.0) => [
                    Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                        DecoratedBox::new().class("skill-editor-badge") => [
                            Center::new().child(Icon::new(MI_CODE).class("skill-editor-badge-icon")),
                        ],
                        DecoratedBox::new().class("grow") => [
                            Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                                Text::new(name.clone()).class("skill-editor-title"),
                                Text::new(if description.is_empty() {
                                    tr!("settings.skills.editor.default_subtitle")
                                } else {
                                    description.clone()
                                }).class("skill-editor-subtitle"),
                            ]
                        ],
                        ToolButton::new(MI_EDIT_NOTE)
                            .tooltip(tr!("settings.skills.edit_meta"))
                            .on_click({
                                let id = id_for_header.clone();
                                let n = name.clone();
                                let d = description_for_edit.clone();
                                move || {
                                    let ctx = use_context::<AppCtx>();
                                    ctx.skills_dialog.set(Some(SkillDialogKind::Edit {
                                        id: id.clone(),
                                        current_name: n.clone(),
                                        current_description: d.clone(),
                                    }));
                                }
                            })
                            .class("skill-editor-action"),
                        ToolButton::new(MI_DELETE)
                            .tooltip(tr!("app.delete"))
                            .on_click({
                                let id = id_for_header.clone();
                                let n = name.clone();
                                move || {
                                    let ctx = use_context::<AppCtx>();
                                    ctx.skills_dialog.set(Some(SkillDialogKind::Delete {
                                        id: id.clone(),
                                        name: n.clone(),
                                    }));
                                }
                            })
                            .class("skill-editor-action danger"),
                    ]
                ]
            ],
            DecoratedBox::new().class("skill-editor-body grow") => [
                CodeEditor::new()
                    .text(initial)
                    .language(Language::Markdown)
                    .show_line_numbers(true)
                    .tab_width(2)
                    .insert_spaces(true)
                    .on_change(move |change| {
                        schedule_autosave(id_for_change.clone(), change.full_text.to_string());
                    })
                    .class("skill-code-editor"),
            ],
        ]
    }
}

fn empty_state(count: usize) -> impl Widget {
    let hint = if count == 0 {
        tr!("settings.skills.empty.no_skills")
    } else {
        trn!("settings.skills.empty.pick", count)
    };
    mgui! {
        Center::new() => [
            Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().class("skill-empty-bubble") => [
                    Center::new().child(Icon::new(MI_MENU_BOOK).class("skill-empty-icon")),
                ],
                Text::new(tr!("settings.skills.empty.title")).class("skill-empty-title"),
                Padding::symmetric(32.0, 0.0).child(
                    Text::new(hint).class("skill-empty-text"),
                ),
            ]
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Дебаунс автосохранения
// ─────────────────────────────────────────────────────────────────────────────
//
// Каждый keystroke в CodeEditor бьёт `on_change`. Чтобы не писать на диск
// на каждое нажатие клавиши, делаем classic debounce:
// 1. Бампаем глобальный generation-counter.
// 2. Параллельно держим Arc<Mutex<(id, content)>> с последним значением.
// 3. Spawn'им задачу, которая ждёт `AUTOSAVE_DEBOUNCE_MS`. Если на момент
//    пробуждения generation не сдвинулся — пишем; иначе тихо выходим.
//
// Запись делается в фоне (`tokio::task::spawn_blocking` не используем —
// небольшие markdown'ы пишутся быстро). `ctx.skills` обновляется на main-
// потоке через `run_on_main_thread`.

static AUTOSAVE_GEN: AtomicU64 = AtomicU64::new(0);

fn schedule_autosave(id: String, content: String) {
    let gen = AUTOSAVE_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    let pending = Arc::new(PendingWrite {
        gen,
        id: id.clone(),
        content,
    });
    spawn(async move {
        tokio::time::sleep(Duration::from_millis(AUTOSAVE_DEBOUNCE_MS)).await;
        if AUTOSAVE_GEN.load(Ordering::Relaxed) != pending.gen {
            return;
        }
        flush(pending);
    });
}

struct PendingWrite {
    gen: u64,
    id: String,
    content: String,
}

fn flush(p: Arc<PendingWrite>) {
    // Сначала обновим ctx.skills на main-потоке: если запись на диск
    // упадёт, в памяти всё равно правильное значение (и попытается
    // записаться снова на следующее keystroke).
    let id = p.id.clone();
    let content = p.content.clone();
    run_on_main_thread(move || {
        let ctx = use_context::<AppCtx>();
        ctx.skills.update(|list| {
            if let Some(s) = list.iter_mut().find(|s| s.id == id) {
                s.content = content.clone();
            }
        });
        // Получим обновлённый skill и пишем на диск (в фоне, чтобы не
        // блокировать UI на медленной FS).
        let snapshot = ctx
            .skills
            .get_untracked()
            .into_iter()
            .find(|s| s.id == id);
        if let Some(skill) = snapshot {
            spawn(async move {
                if let Err(e) = skills::save(&skill) {
                    tracing::warn!(skill = %skill.id, error = %e, "не удалось сохранить скил");
                }
            });
        }
    });
}
