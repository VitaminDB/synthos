//! Сквозной прогон нод SDXL без GUI: те же `on_run`/`evaluate`, что жмёт
//! кнопка Run, — Checkpoint → Text Encoder (промпт + негатив) → (FLUX Empty
//! Latent | Image → SDXL VAE Encode) → Sampler → VAE Decode → Image Save.
//!
//! ```sh
//! cargo run --release --bin sdxl_smoke -- <sdxl-base-1.0.syn|каталог> <out.png> [width height steps]
//! ```
//! env: SDXL_PROMPT, SDXL_NEGATIVE, SDXL_SEED, SDXL_QUANT=dense|mxfp8|nvfp4,
//! SDXL_RESIDENT=1, SDXL_REPEAT=N (повторы: кэш промпта и удержанный UNet),
//! SDXL_INIT=<картинка> [SDXL_DENOISE=0.6] — img2img через SDXL VAE Encode,
//! SDXL_VRAM_GB=7 — балласт в VRAM (модели остаётся столько гигабайт).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use syngui::prelude::RwSignal;
use synthos::pages::node_editor::eval;
use synthos::pages::node_editor::nodes::{flux, image, sdxl};
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

/// Запустить `on_run` ноды и дождаться, пока она освободится; попутно —
/// минимум свободной VRAM.
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
        NodeRuntime::SdxlSampler { loaded_name, .. } | NodeRuntime::SdxlTextEncoder { loaded_name, .. } => {
            loaded_name.get_untracked().unwrap_or_default()
        }
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
        return Err(format!("использование: {} <sdxl-base-1.0.syn|каталог> <out.png> [width height steps]", args[0]));
    }
    let model_path = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let size: Option<(u32, u32)> = match (args.get(3), args.get(4)) {
        (Some(w), Some(h)) => Some((w.parse().map_err(|_| "width")?, h.parse().map_err(|_| "height")?)),
        _ => None,
    };
    let steps: u32 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(30);
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
    let prompt = env("SDXL_PROMPT").unwrap_or_else(|| {
        "a red fox sitting in fresh snow in a birch forest, golden hour, highly detailed photograph".into()
    });
    let negative = env("SDXL_NEGATIVE").unwrap_or_else(|| "blurry, low quality, deformed, watermark, text".into());
    let seed: u64 = env("SDXL_SEED").and_then(|s| s.parse().ok()).unwrap_or(42);
    let repeat: usize = env("SDXL_REPEAT").and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
    let init = env("SDXL_INIT").map(PathBuf::from);
    let denoise: f32 = env("SDXL_DENOISE").and_then(|s| s.parse().ok()).unwrap_or(0.6);

    flux::shared::ensure_kernels_registered();
    let _ballast = env("SDXL_VRAM_GB").and_then(|v| v.parse::<f64>().ok()).map(|gb| {
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
    let n_ckpt = ctx.add_node(NodeKind::SdxlCheckpoint, zero);
    let n_prompt = ctx.add_node(NodeKind::TextView, zero);
    let n_neg = ctx.add_node(NodeKind::TextView, zero);
    let n_enc = ctx.add_node(NodeKind::SdxlTextEncoder, zero);
    let n_smp = ctx.add_node(NodeKind::SdxlSampler, zero);
    let n_dec = ctx.add_node(NodeKind::SdxlVaeDecode, zero);
    let n_save = ctx.add_node(NodeKind::ImageSave, zero);
    for t in [n_enc, n_smp, n_dec] {
        connect(&ctx, n_ckpt, "model", t, "model");
    }
    connect(&ctx, n_prompt, "out", n_enc, "prompt");
    connect(&ctx, n_neg, "out", n_enc, "negative");
    connect(&ctx, n_enc, "conditioning", n_smp, "conditioning");
    connect(&ctx, n_smp, "latent", n_dec, "latent");
    connect(&ctx, n_dec, "image", n_save, "image");
    let n_venc = init.as_ref().map(|src| {
        let n_img = ctx.add_node(NodeKind::ImageLoad, zero);
        let n_venc = ctx.add_node(NodeKind::SdxlVaeEncode, zero);
        connect(&ctx, n_ckpt, "model", n_venc, "model");
        connect(&ctx, n_img, "image", n_venc, "image");
        connect(&ctx, n_venc, "latent", n_smp, "latent");
        if let NodeRuntime::ImageLoad { path, .. } = &*node(&ctx, n_img).runtime.lock().unwrap() {
            path.set(Some(src.clone()));
        }
        if let NodeRuntime::SdxlSampler { denoise: d, .. } = &*node(&ctx, n_smp).runtime.lock().unwrap() {
            d.set(denoise);
        }
        n_venc
    });
    let default_size = if n_venc.is_none() { Some((1024, 1024)) } else { None };
    if let Some((w, h)) = size.or(default_size) {
        let n_lat = ctx.add_node(NodeKind::FluxEmptyLatent, zero);
        match n_venc {
            Some(v) => connect(&ctx, n_lat, "latent", v, "size"),
            None => connect(&ctx, n_lat, "latent", n_smp, "latent"),
        }
        if let NodeRuntime::FluxEmptyLatent { width, height, .. } = &*node(&ctx, n_lat).runtime.lock().unwrap() {
            width.set(w);
            height.set(h);
        }
    }
    if let NodeRuntime::SdxlCheckpoint { model_path: mp, quant_idx, resident, .. } =
        &*node(&ctx, n_ckpt).runtime.lock().unwrap()
    {
        mp.set(Some(model_path.clone()));
        if let Some(q) = env("SDXL_QUANT") {
            quant_idx.set(match q.as_str() {
                "mxfp8" => 1,
                "nvfp4" => 2,
                _ => 0,
            });
        }
        resident.set(env("SDXL_RESIDENT").is_some_and(|v| v == "1"));
    }
    set_text(&ctx, n_prompt, &prompt);
    set_text(&ctx, n_neg, &negative);
    if let NodeRuntime::SdxlSampler { steps: st, seed: sd, .. } = &*node(&ctx, n_smp).runtime.lock().unwrap() {
        st.set(steps);
        sd.set(seed);
    }
    if let NodeRuntime::ImageSave { path, .. } = &*node(&ctx, n_save).runtime.lock().unwrap() {
        path.set(Some(out.clone()));
    }

    eprintln!(
        "sdxl_smoke: {} | {} | шаги {steps} seed {seed} | img2img {}",
        model_path.display(),
        size.map(|(w, h)| format!("{w}×{h}")).unwrap_or_else(|| "1024×1024 / размер картинки".into()),
        init.as_ref().map(|p| format!("{} denoise {denoise}", p.display())).unwrap_or_else(|| "нет".into())
    );
    for round in 1..=repeat {
        eprintln!("— прогон {round}/{repeat}");
        let enc = run_node(&ctx, n_enc, "text-encoder", sdxl::text_encoder::on_run, &mut min_free)?;
        eprintln!("    {}", status(&ctx, n_enc));
        if let Some(v) = n_venc {
            run_node(&ctx, v, "vae-encode", sdxl::vae::encode_on_run, &mut min_free)?;
        }
        let smp = run_node(&ctx, n_smp, "sampler", sdxl::sampler::on_run, &mut min_free)?;
        eprintln!("    {}", status(&ctx, n_smp));
        run_node(&ctx, n_dec, "vae-decode", sdxl::vae::on_run, &mut min_free)?;
        run_node(&ctx, n_save, "image-save", image::save_on_run, &mut min_free)?;
        eprintln!("  итого: энкодер {enc:.1}s, сэмплер {smp:.1}s");
        if let NodeRuntime::SdxlSampler { seed: sd, .. } = &*node(&ctx, n_smp).runtime.lock().unwrap() {
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
    // Без отметки главного потока `set()` из воркера нод шёл бы в чужой
    // thread-local рантайм сигналов вместо очереди главного потока.
    syngui::signal::init_main_thread();
    if let Err(e) = run() {
        eprintln!("sdxl_smoke: ошибка: {e}");
        std::process::exit(1);
    }
}
