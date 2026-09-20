//! KB на настоящих `.syn`-бандлах, без GUI: устаревший путь в конфиге →
//! автопоиск в каталоге моделей → ленивая загрузка → индексация → гибридный
//! поиск с реранком.
//!
//! Зачем тест: именно эта цепочка не работала у пользователя. В конфиге стоял
//! `~/models/bge-m3` (каталога нет), бандлы лежали в каталоге из «AI модели»,
//! а загрузчик требовал каталог-снапшот и говорил «скачай BGE-M3».
//!
//! Бандлы — в `KB_MODELS_DIR` (по умолчанию `~/Storage/syn_models`); нет файлов —
//! тест пропускается. Считает на CPU (~10 с).
//!
//! ```text
//! cargo test --features testing --release --test kb_bundles_e2e -- --nocapture
//! ```

#![cfg(feature = "testing")]

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use synaptix_rag::doc::ChunkConfig;
use synthos::config::KbConfig;
use synthos::kb::ingest::pipeline::{self, IngestJob};
use synthos::kb::ingest::source::DocSource;
use synthos::kb::loader::{self, LoadPlan};
use synthos::kb::models::{self, FoundBy, ModelKind};
use synthos::kb::search::hybrid_search_with_rerank;
use synthos::kb::KbCtx;

fn models_dir() -> PathBuf {
    std::env::var("KB_MODELS_DIR").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Storage/syn_models")
    })
}

#[tokio::test]
async fn stale_config_path_bundles_load_index_and_search() {
    let dir = models_dir();
    if !dir.join("bge-m3.syn").is_file() || !dir.join("bge-reranker-v2-m3.syn").is_file() {
        eprintln!("skip: нет бандлов в {}", dir.display());
        return;
    }
    syngui::signal::allow_signal_reads_on_this_thread();
    synaptix_kernels_cpu::ensure_registered();

    let work = std::env::temp_dir().join(format!("synthos-kb-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(work.join("docs")).unwrap();
    std::fs::write(
        work.join("docs/family.md"),
        "# Брак\n\nБрачный возраст устанавливается в восемнадцать лет для мужчин и женщин. \
         Брак заключается в органах записи актов гражданского состояния.\n",
    )
    .unwrap();
    std::fs::write(
        work.join("docs/borsch.md"),
        "# Борщ\n\nДля приготовления борща понадобятся свёкла, капуста, картофель и мясо. \
         Варить полтора часа на медленном огне.\n",
    )
    .unwrap();

    // Конфиг как у пользователя: пути ведут в никуда.
    let plan = LoadPlan {
        cfg: KbConfig {
            embedder_model_path: "/nonexistent/models/bge-m3".into(),
            reranker_model_path: "/nonexistent/models/bge-reranker-v2-m3.syn".into(),
            ..KbConfig::default()
        },
        dirs: vec![dir.clone()],
    };
    let found = models::discover(&plan.cfg, &plan.dirs);
    assert_eq!(found.embedder.as_ref().unwrap().by, FoundBy::Discovery);
    assert_eq!(found.embedder.as_ref().unwrap().path, dir.join("bge-m3.syn"));
    assert_eq!(found.reranker.as_ref().unwrap().path, dir.join("bge-reranker-v2-m3.syn"));

    let kb = KbCtx::new(work.join("kb"));
    let embedder = loader::embedder_ready(&kb, &plan).await.expect("эмбеддер из бандла");
    assert_eq!(embedder.dim(), 1024);
    assert!(kb.get_embedder().is_some(), "второй вызов получит тот же Arc");
    let reranker = loader::reranker_ready(&kb, &plan).await.expect("реранкер из бандла");
    assert!(reranker.is_some());

    // Токенайзер чанкера — из того же бандла.
    let model = models::find(ModelKind::Embedder, &plan.cfg, &plan.dirs).unwrap();
    let tokenizer =
        tokenizers::Tokenizer::from_bytes(models::read_tokenizer_json(&model.path).unwrap()).unwrap();

    let mut registry = synthos::kb::CollectionRegistry::new(work.join("kb"));
    let meta = registry.create("e2e".into(), "bge-m3".into(), 1024, 512, 64).unwrap();
    let mut store = registry.open_store(&meta.id).unwrap();
    let job = IngestJob {
        job_id: 1,
        collection_id: meta.id.clone(),
        sources: vec![DocSource::Folder { root: work.join("docs"), extra_excludes: Vec::new() }],
        cancel_flag: Arc::new(AtomicBool::new(false)),
    };
    let chunk_cfg = ChunkConfig { target_tokens: 512, overlap_tokens: 64, min_tokens: 8 };
    let outcome =
        pipeline::run_blocking(&job, &mut store, embedder.as_ref(), &chunk_cfg, &tokenizer, 1024, |_| {}, None)
            .expect("индексация");
    assert_eq!(outcome.indexed, 2, "оба файла записаны: {outcome:?}");
    assert!(outcome.failed.is_empty(), "{outcome:?}");
    assert!(outcome.chunks >= 2, "{outcome:?}");
    let stats = store.stats().unwrap();
    assert_eq!(stats.document_count, 2);
    assert!(stats.chunk_count >= 2, "{stats:?}");

    // Второй прогон по тем же файлам ничего не пересчитывает.
    let outcome =
        pipeline::run_blocking(&job, &mut store, embedder.as_ref(), &chunk_cfg, &tokenizer, 1024, |_| {}, None)
            .expect("повторная индексация");
    assert_eq!((outcome.indexed, outcome.skipped), (0, 2), "{outcome:?}");

    let hits = hybrid_search_with_rerank(
        &store,
        embedder.as_ref(),
        reranker.as_deref(),
        "С какого возраста можно жениться?",
        2,
        4,
    )
    .unwrap();
    assert!(!hits.is_empty());
    assert!(hits[0].chunk.source_path.ends_with("family.md"), "первым — закон, а не рецепт: {:?}", hits[0].chunk.source_path);
    assert!(hits[0].rerank_score.is_some(), "выдача прошла реранк");

    let _ = std::fs::remove_dir_all(&work);
}
