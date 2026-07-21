//! Полоса превью прикреплённых файлов над input-bar.
//!
//! Реактивно подписана на `ChatCtx.draft_attachments`: пока список пуст —
//! отдаёт zero-size DecoratedBox (полоса схлопывается, не занимая места).
//! При наличии файлов — горизонтальный ScrollView с карточками 120×120,
//! каждая с миниатюрой (Image::Cover) и close-крестиком в верхнем правом
//! углу.
//!
//! Удаление и добавление — через `crate::chat::attach`. Сами байты картинок
//! живут в blob-CAS, путь до файла строится из `MsgAttachment::rel_path`.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::{ImageFit, ScrollView, Stack};

use crate::chat::{attach, blobs, MsgAttachment};
use crate::context::AppCtx;
use crate::icons::*;

/// Реактивный билдер ленты превью. Используется внутри `Reactive` в
/// `input_panel::view()`. Возвращает строго один корневой `DecoratedBox`,
/// чтобы Reactive-обёртка могла его подменять при изменениях.
pub fn view() -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let chat = use_context::<AppCtx>().chat.clone();
        let atts = chat.draft_attachments.get();

        if atts.is_empty() {
            // zero-size: полоса не существует визуально, но виджет нужен —
            // Reactive должен возвращать один и тот же тип-сигнатуру.
            return DecoratedBox::new().class("attachments-strip-empty");
        }

        let cards: Vec<Box<dyn Widget>> = atts
            .iter()
            .enumerate()
            .map(|(idx, a)| Box::new(card(a, idx)) as Box<dyn Widget>)
            .collect();

        DecoratedBox::new().class("attachments-strip-wrap").child(
            ScrollView::new()
                .horizontal()
                .child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(cards),
                )
                .class("attachments-strip-scroll"),
        )
    }
}

/// Одна карточка-превью: thumbnail + close-крестик в правом верхнем углу.
/// Кликнул на картинку — пока ничего, можно расширить до zoom-предпросмотра.
fn card(att: &MsgAttachment, idx: usize) -> impl Widget {
    let blob_path = blobs::full_path(att).display().to_string();

    DecoratedBox::new().class("attachment-card").child(mgui! {
        Stack::new() => [
            Image::new(blob_path).fit(ImageFit::Cover).class("attachment-thumb"),
            // Слой с close-кнопкой: разворачивается на весь bounds Stack'а и
            // выравнивает дочернюю кнопку в верхний правый угол через Row.
            DecoratedBox::new().class("attachment-card-overlay").child(mgui! {
                Column::new().main_axis_alignment(MainAxisAlignment::Start) => [
                    Row::new().main_axis_alignment(MainAxisAlignment::End) => [
                        ToolButton::new(MI_CLOSE)
                            .on_click(move || attach::remove_at(idx))
                            .class("attachment-card-close"),
                    ]
                ]
            })
        ]
    })
}
