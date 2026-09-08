//! Markdown страницы: плоская форма для агента и служебный хвост с
//! геометрией и свойствами блоков.

use super::*;

/// Снять с блока всё, что уходит в служебный хвост: геометрию — всегда, а
/// у блоков без места под инлайн-атрибуты — все атрибуты (иначе агент
/// увидел бы ```` ```doc-layout ```` с индексами и сдвинул бы их вставкой).
fn strip_sidecar(block: &mut DocBlock) {
    if free::has_inline_attrs(&block.kind) {
        for k in [free::ATTR_X, free::ATTR_Y, free::ATTR_W, free::ATTR_H] {
            block.attrs.remove(k);
        }
    } else {
        block.attrs = Attrs::default();
    }
}

/// Markdown страницы без служебного хвоста — то, что видит и правит агент.
pub fn plain_markdown(md: &str) -> String {
    let mut model = parse_document(md);
    for b in &mut model.blocks {
        strip_sidecar(b);
    }
    serialize_document(&model)
}

/// Текст страницы от агента + атрибуты старых блоков: блок, чей markdown
/// не изменился, остаётся на своём месте холста и в своём оформлении
/// (совпадение по тексту, по порядку, каждый старый блок — один раз).
pub fn with_sidecar(current: &str, new_plain: &str) -> String {
    let old = parse_document(current);
    let old_md: Vec<String> = old.blocks.iter().map(block_markdown).collect();
    let mut used = vec![false; old.blocks.len()];
    let mut fresh = parse_document(new_plain);
    for b in &mut fresh.blocks {
        let key = block_markdown(b);
        let Some(i) = (0..old.blocks.len()).find(|&i| !used[i] && old_md[i] == key) else { continue };
        used[i] = true;
        for (k, val) in old.blocks[i].attrs.0.iter() {
            if b.attrs.get(k).is_none() {
                b.attrs.set(k.clone(), val.clone());
            }
        }
    }
    serialize_document(&fresh)
}

/// Записать новый плоский текст страницы, сохранив атрибуты нетронутых блоков.
pub(super) fn write_page(ctx: NotesCtx, id: &str, new_plain: &str) -> Result<(), String> {
    let current = ctx.page_markdown(id);
    let merged = with_sidecar(&current, new_plain);
    store_markdown(ctx, id, &merged)
}

fn store_markdown(ctx: NotesCtx, id: &str, md: &str) -> Result<(), String> {
    if ctx.set_page_markdown(id, md) {
        Ok(())
    } else {
        Err(format!("page {id} not found"))
    }
}

/// Модель страницы со служебным хвостом — для правок на уровне блоков.
pub(super) fn load_model(ctx: NotesCtx, id: &str) -> DocModel {
    parse_document(&ctx.page_markdown(id))
}

pub(super) fn store_model(ctx: NotesCtx, id: &str, model: &DocModel) -> Result<(), String> {
    store_markdown(ctx, id, &serialize_document(model))
}
