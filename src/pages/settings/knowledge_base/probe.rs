//! Секция «Проверка поиска»: тот же гибридный поиск, что получает модель
//! через `kb_search`, но по одной открытой коллекции и с раскладкой оценок.
//! Отвечает на вопрос «а находит ли оно вообще» без похода в чат.

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::prelude::*;

use crate::context::AppCtx;
use crate::icons::*;
use crate::kb::ctx::ProbeState;
use crate::kb::loader;
use crate::kb::search::{hybrid_search_with_rerank, SearchHit};

const TOP_K: usize = 5;
/// Сколько символов фрагмента показываем в выдаче.
const SNIPPET_CHARS: usize = 360;

pub(super) fn view(collection_id: String) -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    // Выдача относится к прошлой коллекции/запросу — страница открылась заново.
    ctx.kb.probe.set(ProbeState::Idle);

    let query = use_signal(String::new());
    let id_submit = collection_id.clone();
    let id_click = collection_id;

    let input = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            DecoratedBox::new().class("grow kb-min0").child(
                TextField::new()
                    .placeholder(tr!("settings.knowledge_base.probe.placeholder"))
                    .prefix_icon(MI_TRAVEL_EXPLORE)
                    .on_change(move |s| query.set(s.to_string()))
                    .on_submit(move |s| run(&id_submit, s))
                    .class("kb-probe-field"),
            ),
        )
        .child(
            Button::new(tr!("settings.knowledge_base.probe.run"))
                .leading_icon(MI_SEARCH)
                .on_click(move || run(&id_click, &query.get_untracked()))
                .class("kb-btn kb-btn-primary"),
        );

    let results = Reactive::new(move || {
        let ctx = use_context::<AppCtx>();
        match ctx.kb.probe.get() {
            ProbeState::Idle => Vec::new(),
            ProbeState::Running => vec![note(
                Some(Box::new(CircularProgress::new().indeterminate().size(16.0))),
                tr!("settings.knowledge_base.probe.running"),
                "kb-probe-note",
            )],
            ProbeState::Failed(e) => vec![note(None, e, "kb-probe-note error")],
            ProbeState::Done(hits) if hits.is_empty() => vec![note(
                None,
                tr!("settings.knowledge_base.probe.nothing"),
                "kb-probe-note",
            )],
            ProbeState::Done(hits) => hits
                .into_iter()
                .enumerate()
                .map(|(i, h)| hit_row(i + 1, h))
                .collect(),
        }
    });

    let body = Column::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Padding::symmetric(24.0, 18.0).child(input))
        .child(results);
    super::section(tr!("settings.knowledge_base.probe"), None, Box::new(body))
}

fn note(lead: Option<Box<dyn Widget>>, text: String, class: &str) -> Box<dyn Widget> {
    let mut cells: Vec<Box<dyn Widget>> = Vec::new();
    cells.extend(lead);
    cells.push(Box::new(
        DecoratedBox::new()
            .class("grow kb-min0")
            .child(Text::new(text).class(class.to_string())),
    ));
    Box::new(
        DecoratedBox::new().class("kb-probe-row").child(
            Padding::symmetric(24.0, 14.0).child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(cells),
            ),
        ),
    )
}

fn hit_row(rank: usize, hit: SearchHit) -> Box<dyn Widget> {
    let title = hit
        .chunk
        .doc_title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| hit.chunk.source_path.clone());
    let snippet: String = {
        let flat = hit.chunk.text.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut cut: String = flat.chars().take(SNIPPET_CHARS).collect();
        if flat.chars().count() > SNIPPET_CHARS {
            cut.push('…');
        }
        cut
    };

    // Откуда фрагмент взялся в выдаче: позиции в двух каналах и реранк.
    let mut badges: Vec<Box<dyn Widget>> = Vec::new();
    if let Some(r) = hit.bm25_rank {
        badges.push(badge(tr!("settings.knowledge_base.probe.badge.text", rank = r), ""));
    }
    if let Some(r) = hit.cosine_rank {
        badges.push(badge(tr!("settings.knowledge_base.probe.badge.vector", rank = r), ""));
    }
    if let Some(s) = hit.rerank_score {
        badges.push(badge(
            tr!("settings.knowledge_base.probe.badge.rerank", score = format!("{s:.2}")),
            " accent",
        ));
    }

    Box::new(
        DecoratedBox::new().class("kb-probe-row").child(
            Padding::symmetric(24.0, 14.0).child(
                Row::new()
                    .gap(16.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(
                        DecoratedBox::new()
                            .class("kb-probe-rank")
                            .child(Center::new().child(Text::new(rank.to_string()).class("kb-probe-rank-text"))),
                    )
                    .child(
                        DecoratedBox::new().class("grow kb-min0").child(
                            Column::new()
                                .gap(6.0)
                                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                                .child(Text::new(title).elide(Elide::Middle).class("kb-doc-title"))
                                .child(Text::new(snippet).max_lines(4).class("kb-probe-snippet"))
                                .child(Flex::row().wrap().gap(6.0).children(badges)),
                        ),
                    ),
            ),
        ),
    )
}

fn badge(text: String, modifier: &str) -> Box<dyn Widget> {
    Box::new(
        DecoratedBox::new()
            .class(format!("kb-badge{modifier}"))
            .child(Text::new(text).class("kb-badge-text")),
    )
}

/// Запустить пробный поиск. Модели поднимаются сами. Только с главного потока.
fn run(collection_id: &str, raw_query: &str) {
    let query = raw_query.trim().to_string();
    if query.is_empty() {
        return;
    }
    let app = use_context::<AppCtx>();
    let kb = app.kb.clone();
    if kb.probe.get_untracked() == ProbeState::Running {
        return;
    }
    let registry = kb.registry.get_untracked();
    let Some(meta) = registry.get(collection_id) else { return };
    let db_path = meta.db_path(&registry.kb_dir);
    let plan = loader::plan();
    kb.probe.set(ProbeState::Running);

    spawn(async move {
        let outcome = search(&kb, &plan, db_path, query).await;
        run_on_main_thread(move || {
            kb.probe.set(match outcome {
                Ok(hits) => ProbeState::Done(hits),
                Err(e) => ProbeState::Failed(e),
            });
        });
    });
}

async fn search(
    kb: &crate::kb::KbCtx,
    plan: &loader::LoadPlan,
    db_path: std::path::PathBuf,
    query: String,
) -> std::result::Result<Vec<SearchHit>, String> {
    let embedder = loader::embedder_ready(kb, plan).await?;
    // Реранкер — улучшение, а не условие: не поднялся — ищем без него
    // (в выдаче просто не будет его бейджа, а причина — в карточке «Модели»).
    let reranker = loader::reranker_ready(kb, plan).await.unwrap_or(None);
    let multiplier = plan.cfg.reranker_top_k_multiplier;
    tokio::task::spawn_blocking(move || {
        let store = crate::kb::store::Store::open(&db_path).map_err(|e| e.to_string())?;
        hybrid_search_with_rerank(
            &store,
            embedder.as_ref(),
            reranker.as_deref(),
            &query,
            TOP_K,
            multiplier,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
