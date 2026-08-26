//! Карточки вложений: полоса над полем ввода и плитка внутри bubble.
//!
//! Обе поверхности собираются из одной карточки [`card`], отличаясь только
//! размером (класс) и наличием кнопки удаления. Клик по карточке открывает
//! полноэкранный просмотр (`super::media_viewer`).
//!
//! ```text
//! ┌─────────────┐  ← .attachment-card
//! │  превью     │     Image(Cover) для картинки/видео, иконка для остальных
//! │      ⏵ 0:42 │  ← бейдж длительности у видео/аудио
//! │ имя · 2,1 МБ│  ← .attachment-card-caption
//! └──────────×──┘  ← .attachment-card-close (только в strip'е)
//! ```

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::ScrollDirection;

use crate::icons::{
    MI_AUDIOTRACK, MI_CLOSE, MI_DESCRIPTION, MI_HOURGLASS_TOP, MI_INSERT_DRIVE_FILE,
    MI_MOVIE, MI_PLAY_ARROW,
};
use crate::syn_chat::attach::{self, blobs};
use crate::syn_chat::state::{AttachmentKind, MsgAttachment, SynChatCtx, ViewerState};

/// Полоса вложений над input-панелью. Пустой черновик схлопывается в
/// zero-size box, чтобы не держать вертикальный отступ.
pub fn strip() -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let ctx = use_context::<SynChatCtx>();
        let items = ctx.pending_attachments.get();
        let busy = ctx.attach_busy.get();
        if items.is_empty() && busy == 0 {
            return vec![Box::new(DecoratedBox::new().class("attachments-strip-empty"))];
        }

        let mut cards: Vec<Box<dyn Widget>> = items
            .iter()
            .enumerate()
            .map(|(idx, a)| {
                let all = items.clone();
                Box::new(card(a, CardMode::Draft, move || {
                    open_viewer(all.clone(), idx)
                })) as Box<dyn Widget>
            })
            .collect();
        for _ in 0..busy {
            cards.push(Box::new(busy_card()));
        }

        let summary = summary_line(&items, busy);
        vec![Box::new(
            DecoratedBox::new().class("attachments-strip-wrap").child(mgui! {
                Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    ScrollView::new()
                        .direction(ScrollDirection::Horizontal)
                        .class("attachments-strip-scroll")
                        .child(
                            Row::new()
                                .gap(10.0)
                                .cross_axis_alignment(CrossAxisAlignment::Start)
                                .children(cards),
                        ),
                    Text::new(summary).class("attachments-strip-summary"),
                ]
            }),
        )]
    })
}

/// Подпись под полосой: сколько файлов и во сколько vision-токенов они
/// обойдутся. Оценка приблизительная (см. `attach::estimate_vision_tokens`),
/// но именно она отвечает на вопрос «сколько контекста я сейчас потрачу».
fn summary_line(items: &[MsgAttachment], busy: usize) -> String {
    if items.is_empty() {
        return trn!("chat.attach.summary.preparing_only", busy);
    }
    let cap = use_context::<crate::context::AppCtx>()
        .syn_chat_max_image_tokens
        .get_untracked();
    let cap = (cap > 0).then_some(cap);
    let tokens: usize = items
        .iter()
        .map(|a| attach::estimate_vision_tokens(a, cap))
        .sum();
    let total_bytes: u64 = items.iter().map(|a| a.size_bytes).sum();

    let mut s = format!(
        "{} · {}",
        trn!("chat.attach.count", items.len()),
        attach::format_size(total_bytes)
    );
    if tokens > 0 {
        // Тильда, а не «≈»: в интерфейсном шрифте нет U+2248, и глиф
        // молча выпадает — подпись читалась бы как точное число.
        s.push_str(&format!(" · {}", tr!("chat.attach.summary.tokens_suffix", tokens = tokens)));
    }
    if busy > 0 {
        s.push_str(&format!(" · {}", tr!("chat.attach.summary.more_preparing", busy = busy)));
    }
    s
}

/// Плитка вложений внутри пузырька сообщения.
pub fn bubble_grid(attachments: &[MsgAttachment]) -> Box<dyn Widget> {
    if attachments.is_empty() {
        return Box::new(DecoratedBox::new().class("attachments-strip-empty"));
    }
    let items: Vec<MsgAttachment> = attachments.to_vec();
    let cards: Vec<Box<dyn Widget>> = items
        .iter()
        .enumerate()
        .map(|(idx, a)| {
            let all = items.clone();
            let open = move || open_viewer(all.clone(), idx);
            // Медиа играет прямо в ленте: результат прогона смотрят здесь же,
            // а не через модальный просмотрщик (он остаётся по клику).
            if super::media_inline::is_inline(a) {
                super::media_inline::media_card(a, open)
            } else {
                Box::new(card(a, CardMode::Sent, open)) as Box<dyn Widget>
            }
        })
        .collect();

    Box::new(
        DecoratedBox::new().class("msg-attachments").child(
            Flex::new()
                .direction(FlexDirection::Row)
                .wrap()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .children(cards),
        ),
    )
}

/// Где рисуется карточка — от этого зависят класс и наличие крестика.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CardMode {
    /// Черновик над полем ввода: карточку можно убрать.
    Draft,
    /// Уже отправленное сообщение: только просмотр.
    Sent,
}

fn card<F>(a: &MsgAttachment, mode: CardMode, on_open: F) -> impl Widget
where
    F: Fn() + Send + Sync + 'static,
{
    let class = match mode {
        CardMode::Draft => "attachment-card",
        CardMode::Sent => "attachment-card attachment-card-sent",
    };

    let mut layers: Vec<Box<dyn Widget>> = vec![preview_layer(a)];
    if let Some(badge) = duration_badge(a) {
        layers.push(badge);
    }
    layers.push(Box::new(caption(a)));
    if mode == CardMode::Draft {
        layers.push(Box::new(close_overlay(&a.sha256)));
    }

    let stack = Stack::new().fit(StackFit::Expand).children(layers);
    let inner = DecoratedBox::new().class(class).child(stack);

    GestureDetector::new()
        .cursor(syngui::input::CursorIcon::Pointer)
        .on_click(move || on_open())
        .child(inner)
}

/// Превью: картинка для image/video, иконка-заглушка для остальных типов.
fn preview_layer(a: &MsgAttachment) -> Box<dyn Widget> {
    match blobs::preview_path(a) {
        Some(path) => Box::new(
            Image::new(path.display().to_string())
                .fit(ImageFit::Cover)
                .class("attachment-thumb"),
        ),
        None => Box::new(
            DecoratedBox::new().class("attachment-icon-wrap").child(
                Center::new().child(
                    Icon::new(kind_icon(a.kind)).class("attachment-icon"),
                ),
            ),
        ),
    }
}

pub(super) fn kind_icon(kind: AttachmentKind) -> &'static str {
    match kind {
        AttachmentKind::Image => crate::icons::MI_IMAGE_ICON,
        AttachmentKind::Video => MI_MOVIE,
        AttachmentKind::Audio => MI_AUDIOTRACK,
        AttachmentKind::Document => MI_DESCRIPTION,
        AttachmentKind::Other => MI_INSERT_DRIVE_FILE,
    }
}

/// Бейдж «⏵ 1:23» в углу — только там, где есть длительность.
fn duration_badge(a: &MsgAttachment) -> Option<Box<dyn Widget>> {
    if a.duration_ms == 0 {
        return None;
    }
    let text = attach::format_duration(a.duration_ms);
    Some(Box::new(
        DecoratedBox::new()
            .class("attachment-card-badge-wrap")
            .child(mgui! {
                Column::new().main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Start) => [
                    DecoratedBox::new().class("attachment-card-badge") => [
                        Row::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                            Icon::new(MI_PLAY_ARROW).class("attachment-card-badge-icon"),
                            Text::new(text).class("attachment-card-badge-text"),
                        ]
                    ]
                ]
            }),
    ))
}

/// Подпись поверх нижней кромки карточки: имя файла + размер/разрешение.
fn caption(a: &MsgAttachment) -> impl Widget {
    let name = if a.original_name.is_empty() {
        a.kind.label().to_string()
    } else {
        a.original_name.clone()
    };
    let meta = attach::short_meta(a);
    DecoratedBox::new()
        .class("attachment-card-caption-wrap")
        .child(mgui! {
            Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                DecoratedBox::new().class("attachment-card-caption") => [
                    Column::new().gap(1.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                        Text::new(name).class("attachment-card-name"),
                        Text::new(meta).class("attachment-card-meta"),
                    ]
                ]
            ]
        })
}

/// Крестик удаления в правом верхнем углу (только черновик).
fn close_overlay(sha: &str) -> impl Widget {
    let sha = sha.to_string();
    DecoratedBox::new()
        .class("attachment-card-overlay")
        .child(mgui! {
            Row::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Start) => [
                ToolButton::new(MI_CLOSE)
                    .tooltip(tr!("chat.attach.remove.tooltip"))
                    .on_click(move || attach::remove_pending(&sha))
                    .class("attachment-card-close"),
            ]
        })
}

/// Плейсхолдер на время ingest'а: хеширование и ffmpeg занимают секунды.
fn busy_card() -> impl Widget {
    DecoratedBox::new().class("attachment-card attachment-card-busy").child(mgui! {
        Center::new() => [
            Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_HOURGLASS_TOP).class("attachment-icon"),
                Text::new(tr!("chat.attach.busy_card.label")).class("attachment-card-meta"),
            ]
        ]
    })
}

fn open_viewer(items: Vec<MsgAttachment>, index: usize) {
    let ctx = use_context::<SynChatCtx>();
    ctx.viewer.set(Some(ViewerState { items, index }));
}
