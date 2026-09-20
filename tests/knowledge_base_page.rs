//! Settings → «Базы знаний»: страница коллекции и список коллекций.
//!
//! Зачем тест: страница — сплошная раскладка и реактивность, которых сборка не
//! проверяет. Прежняя версия держалась на классах `models-*`, которых нет ни в
//! одном MSS (карточки были невидимы), а строка «Эмбеддер» читала состояние из
//! `Mutex` мимо сигналов и не узнавала, что модель загрузилась.
//!
//! Нужен `TestHarness` из syngui: `cargo test --features testing`.

#![cfg(feature = "testing")]

use std::path::PathBuf;

use syngui::prelude::*;
use syngui::testing::{click_at, TestHarness};
use syngui::widget::ElementId;

use synthos::config::AppConfig;
use synthos::context::AppCtx;
use synthos::kb::ctx::ModelState;
use synthos::kb::ingest::pipeline::{IngestProgress, IngestStage};
use synthos::kb::models::{FoundBy, FoundModel, ModelPaths};
use synthos::kb::store::{ChunkRow, DocumentRow};
use synthos::pages::settings::knowledge_base::{self, collections_panel};

const WIDTH: f32 = 1100.0;
const HEIGHT: f32 = 2400.0;

fn render(widget: Box<dyn Widget>, width: f32) -> TestHarness {
    let mut h = TestHarness::new(widget);
    settle(&mut h, width);
    h
}

fn settle(h: &mut TestHarness, width: f32) {
    syngui::signal::drain_and_run_effects();
    let engine = h.apply_mss(synthos::styles::styles());
    h.rebuild();
    h.apply_styles(&engine);
    h.layout(width, HEIGHT);
}

fn center(h: &TestHarness, id: ElementId) -> Point {
    let b = h.element_bounds(id);
    Point::new(b.origin.x + b.size.width / 2.0, b.origin.y + b.size.height / 2.0)
}

fn right_edge(h: &TestHarness, id: ElementId) -> f32 {
    let b = h.element_bounds(id);
    b.origin.x + b.size.width
}

fn count(h: &TestHarness, class: &str) -> usize {
    h.find_by_class(class).len()
}

fn found(name: &str) -> FoundModel {
    FoundModel {
        path: PathBuf::from("/models").join(name),
        by: FoundBy::Discovery,
        bytes: 2_283_984_138,
    }
}

fn doc(path: &str) -> DocumentRow {
    DocumentRow {
        id: None,
        source_kind: "file".into(),
        source_path: path.into(),
        sha256: format!("sha-{path}"),
        title: None,
        bytes: 4096,
        indexed_at: 1_700_000_000,
    }
}

#[test]
fn knowledge_base_page_sections_states_and_actions() {
    let home = std::env::temp_dir().join(format!("synthos-kb-page-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);
    syngui::signal::allow_signal_reads_on_this_thread();
    syngui::i18n::register_catalogs(&synthos::i18n::CATALOGS);
    // Английский: харнесс меряет текст по байтам, кириллица выходит вдвое шире.
    syngui::i18n::set_language(syngui::i18n::Lang::new("en"));

    let (_theme, ctx) = synthos::build_context();
    provide_context(ctx.clone());
    provide_context(synthos::pages::huggingface::HuggingFaceCtx::new(&AppConfig::default()));
    provide_context(synthos::pages::syn_explorer::SynExplorerCtx::new(&AppConfig::default()));
    let kb = use_context::<AppCtx>().kb;

    // ── Пусто: приглашение с кнопкой, она создаёт и открывает коллекцию ──
    let mut h = render(Box::new(knowledge_base::view()), WIDTH);
    assert_eq!(count(&h, "kb-empty-bubble"), 1, "пустое состояние");
    let create = h.find_by_class("kb-btn-primary");
    assert_eq!(create.len(), 1, "кнопка «Создать коллекцию»");
    h.send_events(&click_at(center(&h, create[0])));
    let id = kb.active_collection_id.get_untracked().expect("коллекция создана и открыта");
    assert_eq!(kb.registry.get_untracked().items.len(), 1);

    // Документы кладём прямо в БД — индексации (и моделей) тест не трогает.
    // Первый — с фрагментом, остальные без: так выглядит документ, чью
    // индексацию оборвали, и страница обязана это показывать.
    let dim = kb.registry.get_untracked().get(&id).unwrap().embedding_dim as usize;
    {
        let mut store = kb.registry.get_untracked().open_store(&id).unwrap();
        for i in 0..30 {
            let doc_id = store
                .upsert_document(&doc(&format!("/home/user/project/docs/chapter-{i:02}.md")))
                .unwrap()
                .0;
            if i == 0 {
                store
                    .insert_chunks(
                        doc_id,
                        &[ChunkRow {
                            ord: 0,
                            text: "chapter zero".into(),
                            start_byte: 0,
                            end_byte: 12,
                            token_count: 2,
                            embedding: vec![0.0; dim],
                        }],
                        dim,
                    )
                    .unwrap();
            }
        }
    }
    kb.registry.update(|r| r.scan());

    // ── Страница коллекции: секции в карточках, ничего не торчит за край ──
    let mut h = render(Box::new(knowledge_base::view()), WIDTH);
    assert_eq!(count(&h, "kb-hero"), 1, "шапка коллекции");
    assert_eq!(count(&h, "kb-stat-chip"), 3, "документы / фрагменты / модель");
    assert_eq!(count(&h, "kb-click-row"), 3, "файлы / папка / веб-страница");
    assert_eq!(count(&h, "settings-section-title"), 4, "источники, документы, поиск, модели");
    let cards = h.find_by_class("settings-card");
    assert_eq!(cards.len(), 5, "шапка + четыре секции");
    for card in &cards {
        let b = h.element_bounds(*card);
        assert!(b.size.height > 40.0, "карточка схлопнута: {b:?}");
        assert!(right_edge(&h, *card) <= WIDTH + 0.5, "карточка за правым краем: {b:?}");
    }
    // Фон и рамка карточки — из темы: раньше класса в MSS не было вовсе.
    let mss = h.element_mss(cards[0]).expect("стили карточки");
    assert!(mss.background_color.is_some(), "у карточки нет фона");
    assert!(mss.border_radius.is_some(), "у карточки нет скругления");

    // Длинный список — частями, с фильтром и кнопкой «Показать все».
    assert_eq!(count(&h, "kb-doc-title"), 25, "первая страница документов");
    assert_eq!(count(&h, "kb-filter-field"), 1, "фильтр появляется на длинном списке");
    // Фрагменты видны в каждой строке, «нет фрагментов» — подсвечено.
    assert_eq!(count(&h, "kb-doc-meta"), 25, "счётчик фрагментов у каждой строки");
    assert_eq!(count(&h, "warn"), 24, "документы без фрагментов помечены");

    // ── Строки моделей ────────────────────────────────────────────────
    // Поиск ещё идёт: ни предупреждения, ни кнопок.
    kb.model_paths_ready.set(false);
    kb.model_paths.set(ModelPaths::default());
    settle(&mut h, WIDTH);
    assert_eq!(count(&h, "kb-desc-warn"), 0, "пока ищем — не пугаем «не найдено»");

    // Не нашли: предупреждение и «Указать файл…» у обеих моделей.
    kb.model_paths.set(ModelPaths { searched: vec![PathBuf::from("/models")], ..Default::default() });
    kb.model_paths_ready.set(true);
    settle(&mut h, WIDTH);
    assert_eq!(count(&h, "kb-desc-warn"), 2, "обе модели не найдены");
    assert_eq!(count(&h, "kb-btn-soft"), 0, "грузить нечего");

    // Нашли: «Загрузить» у обеих, предупреждений нет.
    kb.model_paths.set(ModelPaths {
        embedder: Some(found("bge-m3.syn")),
        reranker: Some(found("bge-reranker-v2-m3.syn")),
        searched: vec![PathBuf::from("/models")],
    });
    settle(&mut h, WIDTH);
    assert_eq!(count(&h, "kb-desc-warn"), 0);
    assert_eq!(count(&h, "kb-btn-soft"), 2, "кнопка «Загрузить» у каждой модели");

    // Состояние приходит сигналом: загрузка → готово → ошибка.
    kb.embedder_state.set(ModelState::Loading);
    settle(&mut h, WIDTH);
    assert_eq!(count(&h, "busy"), 1, "чип «Загрузка…»");
    assert_eq!(count(&h, "kb-btn-soft"), 1, "у загружаемой модели кнопки нет");
    kb.embedder_state.set(ModelState::Ready);
    settle(&mut h, WIDTH);
    assert_eq!(count(&h, "ready"), 1, "чип «В памяти»");
    kb.embedder_state.set(ModelState::Failed("boom".into()));
    settle(&mut h, WIDTH);
    assert_eq!(count(&h, "kb-desc-error"), 1, "текст ошибки вместо пути");
    assert_eq!(count(&h, "kb-btn-soft"), 2, "«Повторить» + «Загрузить» реранкера");
    kb.embedder_state.set(ModelState::Idle);

    // Контролы строк моделей не вылезают за карточку.
    settle(&mut h, WIDTH);
    for btn in h.find_by_class("kb-btn-soft") {
        assert!(right_edge(&h, btn) <= WIDTH - 32.0, "кнопка модели за краем карточки");
    }

    // ── Индексация: строки добавления уступают место прогрессу ─────────
    kb.ingest_progress.set(Some(IngestProgress {
        job_id: 1,
        collection_id: id.clone(),
        stage: IngestStage::Embedding,
        current: 3,
        total: 12,
        current_file: Some("/home/user/project/docs/chapter-03.md".into()),
        error: None,
    }));
    settle(&mut h, WIDTH);
    assert_eq!(count(&h, "kb-click-row"), 0, "во время индексации добавлять нельзя");
    let bars = h.find_by_class("kb-progress-bar");
    assert_eq!(bars.len(), 1, "полоса прогресса");
    assert!(h.element_bounds(bars[0]).size.width > 300.0, "полоса во всю карточку");
    kb.ingest_progress.set(None);
    settle(&mut h, WIDTH);
    assert_eq!(count(&h, "kb-click-row"), 3);

    // ── Удаление документа: строка уходит, счётчик коллекции обновляется ──
    let before = kb.registry.get_untracked().get(&id).unwrap().document_count;
    let first_doc_y = h.element_bounds(h.find_by_class("kb-doc-title")[0]).origin.y;
    let rows_from = |h: &TestHarness, class: &str| -> Vec<ElementId> {
        h.find_by_class(class)
            .into_iter()
            .filter(|e| h.element_bounds(*e).origin.y > first_doc_y - 30.0)
            .collect()
    };
    assert_eq!(rows_from(&h, "kb-icon-btn").len(), 50, "переиндексация + корзина у строки");
    let trash: Vec<ElementId> = rows_from(&h, "danger");
    assert_eq!(trash.len(), 25, "корзина у каждой строки документа");
    h.send_events(&click_at(center(&h, trash[0])));
    settle(&mut h, WIDTH);
    let after = kb.registry.get_untracked().get(&id).unwrap().document_count;
    assert_eq!(after, before - 1, "документ удалён из БД");

    // ── Узкое окно: страница ужимается, а не уезжает за край ───────────
    let h_narrow = render(Box::new(knowledge_base::view()), 620.0);
    for card in h_narrow.find_by_class("settings-card") {
        assert!(right_edge(&h_narrow, card) <= 620.5, "карточка за краем узкого окна");
    }

    // ── Правая панель: строка на коллекцию, «в чате» не открывает её ───
    collections_panel::create_collection();
    let second = kb.active_collection_id.get_untracked().unwrap();
    assert_ne!(second, id);
    kb.active_collection_id.set(Some(id.clone()));
    let mut p = render(Box::new(collections_panel::view()), 300.0);
    assert_eq!(count(&p, "skill-list-row"), 2, "две коллекции");
    assert_eq!(count(&p, "selected"), 1, "открытая подсвечена");
    let toggles = p.find_by_class("kb-coll-chat");
    assert_eq!(toggles.len(), 2);
    for t in &toggles {
        assert!(right_edge(&p, *t) <= 300.5, "кнопка «в чате» за краем панели");
    }
    p.send_events(&click_at(center(&p, toggles[1])));
    assert_eq!(kb.active_in_chat_ids.get_untracked().len(), 1, "коллекция подключена к чату");
    assert_eq!(
        kb.active_collection_id.get_untracked().as_deref(),
        Some(id.as_str()),
        "переключатель «в чате» не меняет открытую коллекцию"
    );
    settle(&mut p, 300.0);
    assert_eq!(count(&p, "kb-coll-in-chat"), 1, "подключённая видна по залитой иконке");

    let _ = std::fs::remove_dir_all(&home);
}
