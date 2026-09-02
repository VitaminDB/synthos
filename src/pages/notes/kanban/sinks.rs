//! Обмен между страницей и досками через drag-подсистему syngui.
//!
//! Блок страницы, взятый за ⋮⋮, редактор объявляет drag'ом дерева типа
//! [`DRAG_TYPE_BLOCK`] (payload — id блока; `DocumentEditor::block_drag_type`).
//! Его принимают `DropArea` колонок доски — они же принимают карточки, — а
//! хост по payload'у забирает markdown блока и удаляет его со страницы
//! ([`take_page_block`]). Обратно: карточка, отпущенная на документ мимо
//! досок (`on_drop_data` редактора), становится блоками страницы в точке
//! дропа ([`drop_on_page`]). Дерево отдаёт дроп самой глубокой цели под
//! курсором, поэтому редактор и доска внутри него не спорят за одно событие.

use syngui::core::Point;
use syngui::input::DragData;
use syngui::widgets::input::document_editor::{BlockId, DocOp};

use super::super::state::{LiveObject, NotesCtx};
use super::view::DRAG_TYPE_CARD;
use super::KanbanHandle;

/// Тип drag-данных блока страницы.
pub const DRAG_TYPE_BLOCK: &str = "notes-doc-block";

/// Markdown блока активной страницы по payload'у drag'а; блок ставится на
/// удаление (`DocOp::DeleteBlock`) — он стал карточкой. `None` — блока нет
/// или он пуст.
pub fn take_page_block(ctx: NotesCtx, payload: &str) -> Option<String> {
    let id = BlockId(payload.trim().parse().ok()?);
    let page = ctx.active_page()?;
    let md = page.handle.block_markdown(id)?;
    if md.trim().is_empty() {
        return None;
    }
    ctx.doc_op(DocOp::DeleteBlock(id));
    Some(md)
}

/// Карточка доски отпущена на документ (мимо колонок): она становится
/// блоками страницы в точке дропа и уходит с доски.
pub fn drop_on_page(ctx: NotesCtx, pos: Point, data: &DragData) -> bool {
    if data.drag_type != DRAG_TYPE_CARD {
        return false;
    }
    let Some((board_id, card_id)) = data.payload.split_once('|') else { return false };
    let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", board_id) else { return false };
    let Some(card) = handle.take_card(card_id) else { return false };
    let _: &KanbanHandle = &handle;
    ctx.doc_op(DocOp::InsertMarkdownAt { at: pos, md: KanbanHandle::card_markdown(&card) });
    true
}
