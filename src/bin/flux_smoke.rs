//! Сквозной прогон нод FLUX без GUI: те же `on_run`/`evaluate`, что жмёт
//! кнопка Run, — Checkpoint → Text Encoder → Empty Latent | Image + VAE
//! Encode → Sampler → VAE Decode → Image Save. Потом кадр подаётся проводом
//! в LTX Image и H3 Keyframe (сами видео-модели не запускаются).
//!
//! ```sh
//! cargo run --release --bin flux_smoke -- <flux.syn|каталог> <out.png> [width height steps]
//! ```
//! env: FLUX_PROMPT, FLUX_SEED, FLUX_QUANT=nvfp4|mxfp8|dense,
//! FLUX_MEMORY_MODE=auto|resident|block_offload, FLUX_RESIDENT=1 (держать
//! трансформер), FLUX_IMAGE=<исходник> + FLUX_DENOISE (img2img),
//! FLUX_REPEAT=N (повторы с тем же промптом: кэш T5 и резидентный DiT).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use syngui::prelude::RwSignal;
use synthos::pages::node_editor::eval;
use synthos::pages::node_editor::nodes::{flux, image};
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

/// Запустить `on_run` ноды и дождаться, пока она освободится.
fn run_node(
    ctx: &NodeEditorCtx,
    id: NodeId,
    name: &str,
    on_run: fn(&NodeInstance, &NodeEditorCtx),
) -> Result<f32, String> {
    refresh(ctx);
    let n = node(ctx, id);
    let rt = n.runtime.lock().unwrap().run_error_signal();
    let error = rt.ok_or_else(|| format!("{name}: у ноды нет сигнала ошибки"))?;
    let meta = synthos::pages::node_editor::registry::meta(n.kind);
    let running: RwSignal<bool> = meta.busy_signal.and_then(|f| f(&n)).ok_or("нет busy-сигнала")?;
    let progress = n.runtime.lock().unwrap().run_progress_signal();
    let t0 = Instant::now();
    on_run(&n, ctx);
    let mut last = -1.0f32;
    loop {
        drain();
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
        std::thread::sleep(Duration::from_millis(100));
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
    if args.len() < 3 {
        return Err(format!("использование: {} <flux.syn|каталог> <out.png> [width height steps]", args[0]));
    }
    let model_path = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let width: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(768);
    let height: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(768);
    let steps: u32 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(20);
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
    let prompt = env("FLUX_PROMPT").unwrap_or_else(|| {
        "a cozy reading nook by a rainy window, warm lamp light, a cat asleep on a knitted blanket, photograph".into()
    });
    let seed: u64 = env("FLUX_SEED").and_then(|s| s.parse().ok()).unwrap_or(42);
    let source = env("FLUX_IMAGE").map(PathBuf::from);
    let denoise: f32 = env("FLUX_DENOISE").and_then(|s| s.parse().ok()).unwrap_or(0.6);
    let repeat: usize = env("FLUX_REPEAT").and_then(|s| s.parse().ok()).unwrap_or(1).max(1);

    flux::shared::ensure_kernels_registered();
    let ctx = NodeEditorCtx::new();
    let zero = syngui::core::Point::new(0.0, 0.0);
    let n_ckpt = ctx.add_node(NodeKind::FluxCheckpoint, zero);
    let n_prompt = ctx.add_node(NodeKind::TextView, zero);
    let n_enc = ctx.add_node(NodeKind::FluxTextEncoder, zero);
    let n_smp = ctx.add_node(NodeKind::FluxSampler, zero);
    let n_dec = ctx.add_node(NodeKind::FluxVaeDecode, zero);
    let n_save = ctx.add_node(NodeKind::ImageSave, zero);
    for t in [n_enc, n_smp, n_dec] {
        connect(&ctx, n_ckpt, "model", t, "model");
    }
    connect(&ctx, n_prompt, "out", n_enc, "prompt");
    connect(&ctx, n_enc, "conditioning", n_smp, "conditioning");
    connect(&ctx, n_smp, "latent", n_dec, "latent");
    connect(&ctx, n_dec, "image", n_save, "image");
    let n_vae_enc = if let Some(src) = &source {
        let n_img = ctx.add_node(NodeKind::ImageLoad, zero);
        let n_venc = ctx.add_node(NodeKind::FluxVaeEncode, zero);
        connect(&ctx, n_ckpt, "model", n_venc, "model");
        connect(&ctx, n_img, "image", n_venc, "image");
        connect(&ctx, n_venc, "latent", n_smp, "latent");
        if let NodeRuntime::ImageLoad { path, .. } = &*node(&ctx, n_img).runtime.lock().unwrap() {
            path.set(Some(src.clone()));
        }
        Some(n_venc)
    } else {
        let n_lat = ctx.add_node(NodeKind::FluxEmptyLatent, zero);
        connect(&ctx, n_lat, "latent", n_smp, "latent");
        if let NodeRuntime::FluxEmptyLatent { width: w, height: h, .. } = &*node(&ctx, n_lat).runtime.lock().unwrap() {
            w.set(width);
            h.set(height);
        }
        None
    };

    if let NodeRuntime::FluxCheckpoint { model_path: mp, quant_idx, memory_mode_idx, resident, .. } =
        &*node(&ctx, n_ckpt).runtime.lock().unwrap()
    {
        mp.set(Some(model_path.clone()));
        if let Some(q) = env("FLUX_QUANT") {
            quant_idx.set(match q.as_str() {
                "nvfp4" => 0,
                "dense" => 2,
                _ => 1,
            });
        }
        if let Some(m) = env("FLUX_MEMORY_MODE") {
            memory_mode_idx.set(match m.as_str() {
                "resident" => 1,
                "block_offload" => 2,
                _ => 0,
            });
        }
        resident.set(env("FLUX_RESIDENT").is_some_and(|v| v == "1"));
    }
    if let NodeRuntime::TextView { output_text, .. } = &*node(&ctx, n_prompt).runtime.lock().unwrap() {
        output_text.set(prompt.clone());
    }
    if let NodeRuntime::FluxSampler { steps: st, seed: sd, denoise: dn, .. } = &*node(&ctx, n_smp).runtime.lock().unwrap() {
        st.set(steps);
        sd.set(seed);
        dn.set(if source.is_some() { denoise } else { 1.0 });
    }
    if let NodeRuntime::ImageSave { path, .. } = &*node(&ctx, n_save).runtime.lock().unwrap() {
        path.set(Some(out.clone()));
    }

    eprintln!(
        "flux_smoke: {} | {}×{} {} шагов seed {seed}{}",
        model_path.display(),
        width,
        height,
        steps,
        if source.is_some() { format!(" | img2img denoise {denoise}") } else { String::new() }
    );
    for round in 1..=repeat {
        eprintln!("— прогон {round}/{repeat}");
        let enc = run_node(&ctx, n_enc, "text-encoder", flux::text_encoder::on_run)?;
        if let Some(v) = n_vae_enc {
            run_node(&ctx, v, "vae-encode", flux::vae::encode_on_run)?;
        }
        let smp = run_node(&ctx, n_smp, "sampler", flux::sampler::on_run)?;
        run_node(&ctx, n_dec, "vae-decode", flux::vae::decode_on_run)?;
        run_node(&ctx, n_save, "image-save", image::save_on_run)?;
        eprintln!("  итого: энкодер {enc:.1}s, сэмплер {smp:.1}s");
        if let NodeRuntime::FluxSampler { seed: sd, .. } = &*node(&ctx, n_smp).runtime.lock().unwrap() {
            sd.set(seed + round as u64);
        }
    }
    let meta = std::fs::metadata(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!("сохранено: {} ({} байт)", out.display(), meta.len());

    // Кадр FLUX проводом в видео-ноды.
    let n_ltx = ctx.add_node(NodeKind::LtxImage, zero);
    let n_kf = ctx.add_node(NodeKind::H3Keyframe, zero);
    connect(&ctx, n_dec, "image", n_ltx, "image");
    connect(&ctx, n_dec, "image", n_kf, "image");
    refresh(&ctx);
    let values = ctx.values.get_untracked();
    let img = values.get(&(n_dec, "image")).and_then(|v| v.as_image()).ok_or("VAE Decode не отдал картинку")?;
    let (t, _, _) = values
        .get(&(n_ltx, "image_cond"))
        .and_then(|v| v.as_ltx_image_cond())
        .ok_or("LTX Image не взял кадр с провода")?;
    let kf = values
        .get(&(n_kf, "keyframe"))
        .and_then(|v| v.as_h3_keyframe())
        .ok_or("H3 Keyframe не взял кадр с провода")?;
    let want = vec![3, img.height as usize, img.width as usize];
    if t.dims() != want.as_slice() || kf.image.dims() != want.as_slice() {
        return Err(format!("размеры кадра: LTX {:?}, H3 {:?}, ждали {want:?}", t.dims(), kf.image.dims()));
    }
    eprintln!("кадр {}×{} дошёл до LTX Image и H3 Keyframe", img.width, img.height);
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("flux_smoke: ошибка: {e}");
        std::process::exit(1);
    }
}
