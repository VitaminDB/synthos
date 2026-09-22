//! Сквозной прогон нод Qwen-Image 2.1 без GUI: те же `on_run`/`evaluate`, что
//! жмёт кнопка Run, — Checkpoint → [Image → Reference …] → Text Encoder →
//! Sampler ([FLUX Empty Latent]) → VAE Decode → Image Save.
//!
//! ```sh
//! cargo run --release --bin qwen_image21_smoke -- <qwen-image-2.1.syn|каталог> <out.png> [steps [width height]]
//! ```
//! env: QWEN21_PROMPT, QWEN21_IMAGES=<картинка[,картинка…]> (референсы, до 10),
//! QWEN21_SEED, QWEN21_CFG (1 — без CFG), QWEN21_NEGATIVE, QWEN21_QUANT=nvfp4|mxfp8|dense,
//! QWEN21_MEMORY_MODE=auto|resident|block_offload, QWEN21_RESOLUTION=512|768|1024|1536|2048,
//! QWEN21_KV_CACHE=0, QWEN21_RESIDENT=1, QWEN21_REPEAT=N, QWEN21_VRAM_GB=7 — балласт
//! в VRAM (модели остаётся столько гигабайт).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use syngui::prelude::RwSignal;
use synthos::pages::node_editor::eval;
use synthos::pages::node_editor::nodes::{flux, image, qwen_image21};
use synthos::pages::node_editor::state::NodeEditorCtx;
use synthos::pages::node_editor::types::{Connection, NodeId, NodeInstance, NodeKind, NodeRuntime};

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

fn connect(ctx: &NodeEditorCtx, from: NodeId, fp: &'static str, to: NodeId, tp: &'static str) {
    ctx.connections.update(|c| c.push(Connection { from_node: from, from_port: fp, to_node: to, to_port: tp }));
}

fn free_vram() -> usize {
    synaptix_core::device::cuda::mem_info(0).map(|(f, _)| f).unwrap_or(0)
}

fn run_node(
    ctx: &NodeEditorCtx,
    id: NodeId,
    name: &str,
    on_run: fn(&NodeInstance, &NodeEditorCtx),
    min_free: &mut usize,
) -> Result<f32, String> {
    refresh(ctx);
    let n = node(ctx, id);
    let error = n.runtime.lock().unwrap().run_error_signal().ok_or_else(|| format!("{name}: нет сигнала ошибки"))?;
    let meta = synthos::pages::node_editor::registry::meta(n.kind);
    let running: RwSignal<bool> = meta.busy_signal.and_then(|f| f(&n)).ok_or("нет busy-сигнала")?;
    let progress = n.runtime.lock().unwrap().run_progress_signal();
    let t0 = Instant::now();
    on_run(&n, ctx);
    let mut last = -1.0f32;
    loop {
        drain();
        *min_free = (*min_free).min(free_vram());
        if let Some(e) = error.get_untracked() {
            return Err(format!("{name}: {e}"));
        }
        if !running.get_untracked() {
            break;
        }
        if let Some(p) = progress {
            let pct = p.get_untracked();
            if (pct - last).abs() > 0.09 {
                eprintln!("  [{name}] {:.0}% ({:.1}s)", pct * 100.0, t0.elapsed().as_secs_f32());
                last = pct;
            }
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

fn status(ctx: &NodeEditorCtx, id: NodeId) -> String {
    match &*node(ctx, id).runtime.lock().unwrap() {
        NodeRuntime::QwenImage21Sampler { loaded_name, .. }
        | NodeRuntime::QwenImage21TextEncoder { loaded_name, .. }
        | NodeRuntime::QwenImage21Reference { loaded_name, .. } => loaded_name.get_untracked().unwrap_or_default(),
        _ => String::new(),
    }
}

fn set_text(ctx: &NodeEditorCtx, id: NodeId, text: &str) {
    if let NodeRuntime::TextView { output_text, .. } = &*node(ctx, id).runtime.lock().unwrap() {
        output_text.set(text.to_string());
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        return Err(format!("использование: {} <qwen-image-2.1.syn|каталог> <out.png> [steps [width height]]", args[0]));
    }
    let model_path = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let steps: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    let size: Option<(u32, u32)> = match (args.get(4), args.get(5)) {
        (Some(w), Some(h)) => Some((w.parse().map_err(|_| "width")?, h.parse().map_err(|_| "height")?)),
        _ => None,
    };
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
    let images: Vec<PathBuf> =
        env("QWEN21_IMAGES").map(|s| s.split(',').map(PathBuf::from).collect()).unwrap_or_default();
    let prompt = env("QWEN21_PROMPT").unwrap_or_else(|| {
        if images.is_empty() {
            "A neon shop sign that reads \"QWEN IMAGE 2.1\", rainy night, reflections on wet pavement".into()
        } else {
            "Replace the background with a sunset beach; keep the subject, pose and clothing unchanged".into()
        }
    });
    let seed: u64 = env("QWEN21_SEED").and_then(|s| s.parse().ok()).unwrap_or(42);
    let cfg: f32 = env("QWEN21_CFG").and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let negative = env("QWEN21_NEGATIVE");
    let repeat: usize = env("QWEN21_REPEAT").and_then(|s| s.parse().ok()).unwrap_or(1).max(1);

    flux::shared::ensure_kernels_registered();
    let _ballast = env("QWEN21_VRAM_GB").and_then(|v| v.parse::<f64>().ok()).map(|gb| {
        let keep = (gb * 1e9) as usize;
        let bytes = free_vram().saturating_sub(keep);
        eprintln!("балласт {:.1} ГБ → модели {:.1} ГБ", bytes as f64 / 1e9, keep as f64 / 1e9);
        synaptix_core::tensor::Tensor::zeros(
            (bytes.max(1),),
            synaptix_core::dtype::DType::U8,
            synaptix_core::device::Device::Cuda(0),
        )
        .expect("балласт")
    });
    let avail0 = free_vram();
    let mut min_free = avail0;

    let ctx = NodeEditorCtx::new();
    let zero = syngui::core::Point::new(0.0, 0.0);
    let n_ckpt = ctx.add_node(NodeKind::QwenImage21Checkpoint, zero);
    let n_prompt = ctx.add_node(NodeKind::TextView, zero);
    let n_enc = ctx.add_node(NodeKind::QwenImage21TextEncoder, zero);
    let n_smp = ctx.add_node(NodeKind::QwenImage21Sampler, zero);
    let n_dec = ctx.add_node(NodeKind::QwenImage21VaeDecode, zero);
    let n_save = ctx.add_node(NodeKind::ImageSave, zero);
    for t in [n_enc, n_smp, n_dec] {
        connect(&ctx, n_ckpt, "model", t, "model");
    }
    connect(&ctx, n_prompt, "out", n_enc, "prompt");
    connect(&ctx, n_enc, "conditioning", n_smp, "conditioning");
    connect(&ctx, n_smp, "latent", n_dec, "latent");
    connect(&ctx, n_dec, "image", n_save, "image");
    if let Some(neg) = &negative {
        let n_neg = ctx.add_node(NodeKind::TextView, zero);
        connect(&ctx, n_neg, "out", n_enc, "negative");
        set_text(&ctx, n_neg, neg);
    }
    let mut refs: Vec<NodeId> = Vec::new();
    for (i, src) in images.iter().enumerate() {
        let n_img = ctx.add_node(NodeKind::ImageLoad, zero);
        let n_ref = ctx.add_node(NodeKind::QwenImage21Reference, zero);
        connect(&ctx, n_ckpt, "model", n_ref, "model");
        connect(&ctx, n_img, "image", n_ref, "image");
        if i > 0 {
            connect(&ctx, refs[i - 1], "references", n_ref, "references");
        }
        if let NodeRuntime::ImageLoad { path, .. } = &*node(&ctx, n_img).runtime.lock().unwrap() {
            path.set(Some(src.clone()));
        }
        refs.push(n_ref);
    }
    if let Some(last) = refs.last() {
        connect(&ctx, *last, "references", n_enc, "references");
        connect(&ctx, *last, "references", n_smp, "references");
    }
    if let Some((w, h)) = size {
        let n_lat = ctx.add_node(NodeKind::FluxEmptyLatent, zero);
        connect(&ctx, n_lat, "latent", n_smp, "latent");
        if let NodeRuntime::FluxEmptyLatent { width, height, .. } = &*node(&ctx, n_lat).runtime.lock().unwrap() {
            width.set(w);
            height.set(h);
        }
    }
    if let NodeRuntime::QwenImage21Checkpoint { model_path: mp, quant_idx, memory_mode_idx, resolution_idx, resident, .. } =
        &*node(&ctx, n_ckpt).runtime.lock().unwrap()
    {
        mp.set(Some(model_path.clone()));
        if let Some(q) = env("QWEN21_QUANT") {
            quant_idx.set(match q.as_str() {
                "mxfp8" => 1,
                "dense" => 2,
                _ => 0,
            });
        }
        if let Some(m) = env("QWEN21_MEMORY_MODE") {
            memory_mode_idx.set(match m.as_str() {
                "resident" => 1,
                "block_offload" => 2,
                _ => 0,
            });
        }
        if let Some(r) = env("QWEN21_RESOLUTION") {
            if let Some(i) = qwen_image21::RESOLUTION_OPTIONS.iter().position(|o| *o == r) {
                resolution_idx.set(i);
            }
        }
        resident.set(env("QWEN21_RESIDENT").is_some_and(|v| v == "1"));
    }
    set_text(&ctx, n_prompt, &prompt);
    if let NodeRuntime::QwenImage21Sampler { steps: st, seed: sd, cfg: c, kv_cache, .. } =
        &*node(&ctx, n_smp).runtime.lock().unwrap()
    {
        st.set(steps);
        sd.set(seed);
        c.set(cfg);
        kv_cache.set(!env("QWEN21_KV_CACHE").is_some_and(|v| v == "0"));
    }
    if let NodeRuntime::ImageSave { path, .. } = &*node(&ctx, n_save).runtime.lock().unwrap() {
        path.set(Some(out.clone()));
    }

    eprintln!(
        "qwen_image21_smoke: {} | референсов {} | {} | шаги {steps} (0 — по модели) cfg {cfg} seed {seed}",
        model_path.display(),
        images.len(),
        size.map(|(w, h)| format!("{w}×{h}")).unwrap_or_else(|| "размер по разрешению чекпойнта".into()),
    );
    for round in 1..=repeat {
        eprintln!("— прогон {round}/{repeat}");
        for (i, r) in refs.iter().enumerate() {
            run_node(&ctx, *r, &format!("reference-{}", i + 1), qwen_image21::reference::on_run, &mut min_free)?;
            eprintln!("    {}", status(&ctx, *r));
        }
        let enc = run_node(&ctx, n_enc, "text-encoder", qwen_image21::text_encoder::on_run, &mut min_free)?;
        eprintln!("    {}", status(&ctx, n_enc));
        let smp = run_node(&ctx, n_smp, "sampler", qwen_image21::sampler::on_run, &mut min_free)?;
        eprintln!("    {}", status(&ctx, n_smp));
        run_node(&ctx, n_dec, "vae-decode", qwen_image21::vae::on_run, &mut min_free)?;
        run_node(&ctx, n_save, "image-save", image::save_on_run, &mut min_free)?;
        eprintln!("  итого: энкодер {enc:.1}s, сэмплер {smp:.1}s");
        if let NodeRuntime::QwenImage21Sampler { seed: sd, .. } = &*node(&ctx, n_smp).runtime.lock().unwrap() {
            sd.set(seed + round as u64);
        }
    }
    let meta = std::fs::metadata(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!(
        "сохранено: {} ({} байт); минимум свободной VRAM {:.2} ГБ (пик ≈ {:.2} ГБ из {:.2})",
        out.display(),
        meta.len(),
        min_free as f64 / 1e9,
        avail0.saturating_sub(min_free) as f64 / 1e9,
        avail0 as f64 / 1e9
    );
    Ok(())
}

fn main() {
    syngui::signal::init_main_thread();
    if let Err(e) = run() {
        eprintln!("qwen_image21_smoke: ошибка: {e}");
        std::process::exit(1);
    }
}
