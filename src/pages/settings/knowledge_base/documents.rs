//! Секция «Документы»: что проиндексировано в коллекции.
//!
//! В строке видно число фрагментов: ноль значит, что искать по документу
//! нечем (индексацию оборвали или она упала) — такая строка подсвечена, и
//! рядом с ней кнопка «проиндексировать заново».
//!
//! Список перечитывается из БД по `registry` (счётчики после индексации) и
//! `documents_rev` (удаление документа). Длинный список показывается частями:
//! тысяча строк в одной колонке — это и лаг раскладки, и бесполезная простыня.

use syngui::prelude::*;
use syngui::trn;

use crate::context::AppCtx;
use crate::icons::*;
use crate::kb::ingest::source::DocSource;
use crate::kb::store::{DocumentInfo, DocumentRow};

/// Сколько строк видно, пока не нажали «Показать все».
const PAGE: usize = 25;
/// Поле фильтра появляется, когда список уже не окинуть взглядом.
const FILTER_FROM: usize = 8;

pub(super) fn view(collection_id: String) -> Box<dyn Widget> {
    let filter = use_signal(String::new());
    let expanded = use_signal(false);

    let id_head = collection_id.clone();
    // Фильтр — в заголовке секции; сам TextField вне списка, чтобы
    // перерисовка строк не сбрасывала в нём каретку.
    let trailing = Reactive::new(move || {
        let ctx = use_context::<AppCtx>();
        let count = ctx
            .kb
            .registry
            .get()
            .get(&id_head)
            .map(|m| m.document_count)
            .unwrap_or(0);
        if (count as usize) < FILTER_FROM {
            return Vec::new();
        }
        vec![Box::new(
            TextField::with_text(filter.get_untracked())
                .placeholder(tr!("settings.knowledge_base.documents.filter"))
                .prefix_icon(MI_SEARCH)
                .width(260.0)
                .on_change(move |s| filter.set(s.to_string()))
                .class("kb-filter-field"),
        ) as Box<dyn Widget>]
    });

    let body = Reactive::new(move || {
        let ctx = use_context::<AppCtx>();
        let registry = ctx.kb.registry.get();
        ctx.kb.documents_rev.get();
        let docs = registry
            .open_store(&collection_id)
            .ok()
            .and_then(|s| s.list_documents_with_counts().ok())
            .unwrap_or_default();
        vec![list(&collection_id, docs, &filter.get(), expanded)]
    });

    super::section(
        tr!("settings.knowledge_base.documents"),
        Some(Box::new(trailing)),
        Box::new(body),
    )
}

fn list(
    collection_id: &str,
    docs: Vec<DocumentInfo>,
    filter: &str,
    expanded: RwSignal<bool>,
) -> Box<dyn Widget> {
    if docs.is_empty() {
        return placeholder(MI_DESCRIPTION, tr!("settings.knowledge_base.documents.empty"));
    }
    let needle = filter.trim().to_lowercase();
    let matched: Vec<DocumentInfo> = docs
        .into_iter()
        .filter(|d| {
            needle.is_empty()
                || d.row.source_path.to_lowercase().contains(&needle)
                || d.row.title.as_deref().is_some_and(|t| t.to_lowercase().contains(&needle))
        })
        .collect();
    if matched.is_empty() {
        return placeholder(MI_SEARCH, tr!("settings.knowledge_base.documents.no_match"));
    }

    let total = matched.len();
    let shown = if expanded.get() { total } else { total.min(PAGE) };
    let mut rows: Vec<Box<dyn Widget>> = matched
        .into_iter()
        .take(shown)
        .map(|d| doc_row(collection_id, d))
        .collect();
    if shown < total {
        rows.push(Box::new(
            Padding::symmetric(24.0, 12.0).child(
                Row::new().main_axis_alignment(MainAxisAlignment::Center).child(
                    Button::new(tr!("settings.knowledge_base.documents.show_all", n = total))
                        .trailing_icon(MI_EXPAND_MORE)
                        .on_click(move || expanded.set(true))
                        .class("kb-btn"),
                ),
            ),
        ));
    }
    Box::new(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    )
}

fn placeholder(icon: &str, text: String) -> Box<dyn Widget> {
    Box::new(
        Padding::symmetric(24.0, 28.0).child(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icon.to_string()).class("kb-placeholder-icon"))
                .child(Text::new(text).class("kb-placeholder-text")),
        ),
    )
}

fn doc_row(collection_id: &str, info: DocumentInfo) -> Box<dyn Widget> {
    let doc = info.row;
    let title = doc
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| file_name(&doc.source_path));
    let size = crate::models::human_bytes(doc.bytes.max(0) as u64);
    let empty = info.chunk_count <= 0;
    let fragments = if empty {
        tr!("settings.knowledge_base.documents.no_fragments")
    } else {
        trn!("settings.knowledge_base.documents.fragments", info.chunk_count as u64)
    };
    let meta_class = if empty { "kb-doc-meta warn" } else { "kb-doc-meta" };

    let mut cells: Vec<Box<dyn Widget>> = vec![
        super::row_icon(if empty { MI_SYNC_PROBLEM } else { kind_icon(&doc) }),
        Box::new(
            DecoratedBox::new().class("grow kb-min0").child(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Text::new(title).max_lines(1).class("kb-doc-title"))
                    .child(
                        Text::new(doc.source_path.clone())
                            .elide(Elide::Middle)
                            .class("kb-doc-path"),
                    ),
            ),
        ),
        Box::new(
            Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .child(Text::new(size).class("kb-doc-size"))
                .child(Text::new(fragments).class(meta_class)),
        ),
    ];
    let source = reindex_source(&doc);
    let cid_reindex = collection_id.to_string();
    cells.push(Box::new(
        ToolButton::new(MI_AUTORENEW)
            .tooltip(tr!("settings.knowledge_base.documents.reindex"))
            .on_click(move || reindex(cid_reindex.clone(), source.clone()))
            .class("kb-icon-btn"),
    ));
    if let Some(doc_id) = doc.id {
        let cid = collection_id.to_string();
        cells.push(Box::new(
            ToolButton::new(MI_DELETE)
                .tooltip(tr!("settings.knowledge_base.documents.delete"))
                .on_click(move || delete_doc(&cid, doc_id))
                .class("kb-icon-btn danger"),
        ));
    }
    Box::new(
        DecoratedBox::new().class("settings-row").child(
            Padding::symmetric(24.0, 12.0).child(
                Row::new()
                    .gap(16.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(cells),
            ),
        ),
    )
}

/// Из чего документ индексировался — чтобы повторить это одной кнопкой.
fn reindex_source(doc: &DocumentRow) -> DocSource {
    match doc.source_kind.as_str() {
        "url" => DocSource::Url(doc.source_path.clone()),
        "pdf" => DocSource::Pdf(doc.source_path.clone().into()),
        _ => DocSource::File(doc.source_path.clone().into()),
    }
}

/// Индексация того же источника ещё раз. Неизменившийся файл с фрагментами
/// конвейер пропустит сам; пустой документ (оборванная индексация) —
/// переиндексирует.
fn reindex(collection_id: String, source: DocSource) {
    let app = use_context::<AppCtx>();
    if let DocSource::File(p) | DocSource::Pdf(p) = &source {
        if !p.is_file() {
            app.notifications.error(tr!(
                "settings.knowledge_base.documents.reindex_missing",
                path = p.display()
            ));
            return;
        }
    }
    super::launch(collection_id, vec![source]);
}

fn kind_icon(doc: &DocumentRow) -> &'static str {
    let path = doc.source_path.to_lowercase();
    if path.starts_with("http://") || path.starts_with("https://") {
        return MI_LANGUAGE;
    }
    match path.rsplit('.').next().unwrap_or("") {
        "pdf" => MI_PICTURE_AS_PDF,
        "md" | "markdown" | "txt" | "html" | "htm" => MI_DESCRIPTION,
        _ => MI_CODE,
    }
}

fn file_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn delete_doc(collection_id: &str, doc_id: i64) {
    let app = use_context::<AppCtx>();
    // Параллельная запись в ту же БД из ingest-потока — не трогаем.
    if app.kb.ingest_progress.get_untracked().is_some() {
        app.notifications.warning(tr!("kb.runner.busy"));
        return;
    }
    let result = app
        .kb
        .registry
        .get_untracked()
        .open_store(collection_id)
        .and_then(|store| store.delete_document(doc_id));
    match result {
        Ok(()) => {
            // Счётчики коллекции — из БД; `scan` заодно дёрнет подписчиков.
            app.kb.registry.update(|reg| reg.scan());
            app.kb.documents_rev.update(|r| *r += 1);
        }
        Err(e) => app
            .notifications
            .error(tr!("settings.knowledge_base.documents.delete_failed", error = e)),
    }
}
