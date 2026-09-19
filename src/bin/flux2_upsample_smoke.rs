//! Сквозной прогон встроенного шаблона «FLUX.2: LLM Upsampling» без GUI:
//! шаблон грузится в граф нод так же, как при открытии из каталога
//! (`load_into_ctx`), подставляются бандлы, и ноды запускаются по порядку —
//! LLM (переписывает промпт) → Text View → FLUX.2 Text Encoder → Sampler →
//! VAE Decode → Image Save.
//!
//! ```sh
//! cargo run --release --bin flux2_upsample_smoke -- <flux.2-*.syn> <llm.syn> <out.png> ["короткий промпт"]
//! ```
//! env: FLUX2_UPSAMPLE_STORAGE=0..4 — хранение весов LLM в Syn Checkpoint
//! (Auto, F16, BF16, FP8, NVFP4) вместо шаблонного NVFP4.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use syngui::prelude::RwSignal;
use synthos::pages::node_editor::eval;
use synthos::pages::node_editor::nodes::{flux, flux2, image, llm};
use synthos::pages::node_editor::state::NodeEditorCtx;
use synthos::pages::node_editor::types::{NodeId, NodeInstance, NodeKind, NodeRuntime};

fn drain() {
    syngui::async_runtime::drain_main_thread_callbacks();
}

fn refresh(ctx: &NodeEditorCtx) {
    drain();
    let map = eval::evaluate_graph(&ctx.nodes.get_untracked(), &ctx.connections.get_untracked(), false);
    ctx.values.set(map);
}

fn node(ctx: &NodeEditorCtx, id: NodeId) -> NodeInstance {
    ctx.nodes.get_untracked().iter().find(|n| n.id == id).cloned().expect("нода найдена")
}

/// Ноды шаблона по виду; у Text View их две — по порядку в шаблоне.
fn ids_of(ctx: &NodeEditorCtx, kind: NodeKind) -> Vec<NodeId> {
    ctx.nodes.get_untracked().iter().filter(|n| n.kind == kind).map(|n| n.id).collect()
}

/// Запустить ноду и дождаться, пока она освободится.
fn run_node(ctx: &NodeEditorCtx, id: NodeId, name: &str, on_run: fn(&NodeInstance, &NodeEditorCtx)) -> Result<f32, String> {
    refresh(ctx);
    let n = node(ctx, id);
    let error = n.runtime.lock().unwrap().run_error_signal().ok_or_else(|| format!("{name}: нет сигнала ошибки"))?;
    let meta = synthos::pages::node_editor::registry::meta(n.kind);
    let running: RwSignal<bool> = meta.busy_signal.and_then(|f| f(&n)).ok_or("нет busy-сигнала")?;
    let t0 = Instant::now();
    on_run(&n, ctx);
    loop {
        drain();
        if let Some(e) = error.get_untracked() {
            return Err(format!("{name}: {e}"));
        }
        if !running.get_untracked() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    drain();
    if let Some(e) = error.get_untracked() {
        return Err(format!("{name}: {e}"));
    }
    let s = t0.elapsed().as_secs_f32();
    eprintln!("  [{name}] готово за {s:.1}s");
    Ok(s)
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        return Err(format!("использование: {} <flux.2-*.syn> <llm.syn> <out.png> [\"промпт\"]", args[0]));
    }
    let (flux_path, llm_path, out) = (PathBuf::from(&args[1]), PathBuf::from(&args[2]), PathBuf::from(&args[3]));

    flux::shared::ensure_kernels_registered();
    let t = synthos::templates::list_all()
        .into_iter()
        .find(|t| t.id == "builtin-flux2-llm-upsampling")
        .ok_or("нет шаблона builtin-flux2-llm-upsampling")?;
    let ctx = NodeEditorCtx::new();
    synthos::templates::convert::load_into_ctx(&ctx, &t);
    drain();

    let one = |k: NodeKind| ids_of(&ctx, k).first().copied().ok_or(format!("в шаблоне нет {k:?}"));
    let texts = ids_of(&ctx, NodeKind::TextView);
    let (n_prompt, n_upsampled) = (texts[0], texts[1]);
    let (n_ckpt, n_syn, n_llm) = (one(NodeKind::Flux2Checkpoint)?, one(NodeKind::SynCheckpoint)?, one(NodeKind::Llm)?);
    let (n_enc, n_smp, n_dec, n_save) = (
        one(NodeKind::Flux2TextEncoder)?,
        one(NodeKind::Flux2Sampler)?,
        one(NodeKind::Flux2VaeDecode)?,
        one(NodeKind::ImageSave)?,
    );

    if let NodeRuntime::Flux2Checkpoint { model_path, .. } = &*node(&ctx, n_ckpt).runtime.lock().unwrap() {
        model_path.set(Some(flux_path.clone()));
    }
    if let NodeRuntime::SynCheckpoint { model_path, storage_idx, .. } = &*node(&ctx, n_syn).runtime.lock().unwrap() {
        model_path.set(Some(llm_path.clone()));
        // FLUX2_UPSAMPLE_STORAGE=0..4 (Auto, F16, BF16, FP8, NVFP4) — вместо
        // того, что стоит в шаблоне.
        if let Some(i) = std::env::var("FLUX2_UPSAMPLE_STORAGE").ok().and_then(|v| v.parse().ok()) {
            storage_idx.set(i);
        }
    }
    if let Some(p) = args.get(4) {
        if let NodeRuntime::TextView { output_text, .. } = &*node(&ctx, n_prompt).runtime.lock().unwrap() {
            output_text.set(p.clone());
        }
    }
    if let NodeRuntime::ImageSave { path, .. } = &*node(&ctx, n_save).runtime.lock().unwrap() {
        path.set(Some(out.clone()));
    }
    let text_of = |id: NodeId| match &*node(&ctx, id).runtime.lock().unwrap() {
        NodeRuntime::TextView { output_text, .. } => output_text.get_untracked(),
        _ => String::new(),
    };
    eprintln!("flux2_upsample_smoke: {} + {}\n  промпт: {:?}", flux_path.display(), llm_path.display(), text_of(n_prompt));

    let llm_s = run_node(&ctx, n_llm, "llm", llm::start)?;
    refresh(&ctx);
    refresh(&ctx);
    let upsampled = text_of(n_upsampled);
    eprintln!("  переписанный промпт ({} симв.):\n{upsampled}\n", upsampled.chars().count());
    if upsampled.trim().is_empty() {
        return Err("LLM вернула пустой промпт".into());
    }
    let enc = run_node(&ctx, n_enc, "text-encoder", flux2::text_encoder::on_run)?;
    let smp = run_node(&ctx, n_smp, "sampler", flux2::sampler::on_run)?;
    run_node(&ctx, n_dec, "vae-decode", flux2::vae::on_run)?;
    run_node(&ctx, n_save, "image-save", image::save_on_run)?;
    let meta = std::fs::metadata(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!(
        "сохранено: {} ({} байт); LLM {llm_s:.1}s, энкодер {enc:.1}s, сэмплер {smp:.1}s",
        out.display(),
        meta.len()
    );
    Ok(())
}

fn main() {
    syngui::signal::init_main_thread();
    if let Err(e) = run() {
        eprintln!("flux2_upsample_smoke: ошибка: {e}");
        std::process::exit(1);
    }
}
