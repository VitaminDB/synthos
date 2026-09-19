//! Сквозной прогон нод FLUX.2 без GUI: те же `on_run`/`evaluate`, что жмёт
//! кнопка Run, — Checkpoint → Text Encoder → (FLUX Empty Latent | Image →
//! Reference [→ Reference] | Image → VAE Encode) → Sampler → VAE Decode →
//! Image Save.
//!
//! ```sh
//! cargo run --release --bin flux2_smoke -- <flux.2-*.syn|каталог> <out.png> [width height steps]
//! ```
//! env: FLUX2_PROMPT, FLUX2_SEED, FLUX2_QUANT=nvfp4|mxfp8|dense,
//! FLUX2_MEMORY_MODE=auto|resident|block_offload, FLUX2_RESIDENT=1,
//! FLUX2_EDIT=<картинка> [+ FLUX2_EDIT2=<вторая>] — правка по референсам (без
//! width/height размер берётся с первого референса), FLUX2_REPEAT=N (повторы:
//! кэш промпта и резидентный DiT), FLUX2_VRAM_GB=7 — балласт в VRAM, модели
//! остаётся столько гигабайт (проверка «как на малой карте»),
//! FLUX2_INIT=<картинка> [FLUX2_DENOISE=0.6] — img2img через FLUX.2 VAE Encode
//! (width/height, если заданы, идут в его вход size).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use syngui::prelude::RwSignal;
use synthos::pages::node_editor::eval;
use synthos::pages::node_editor::nodes::{flux, flux2, image};
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
        NodeRuntime::Flux2Sampler { loaded_name, .. }
        | NodeRuntime::Flux2TextEncoder { loaded_name, .. }
        | NodeRuntime::Flux2Reference { loaded_name, .. } => loaded_name.get_untracked().unwrap_or_default(),
        _ => String::new(),
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        return Err(format!("использование: {} <flux.2-*.syn|каталог> <out.png> [width height steps]", args[0]));
    }
    let model_path = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let size: Option<(u32, u32)> = match (args.get(3), args.get(4)) {
        (Some(w), Some(h)) => Some((w.parse().map_err(|_| "width")?, h.parse().map_err(|_| "height")?)),
        _ => None,
    };
    let steps: u32 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
    let prompt = env("FLUX2_PROMPT").unwrap_or_else(|| {
        "A cozy reading nook by a rainy window at dusk: a ginger cat asleep on a chunky knitted blanket, \
         a brass floor lamp casting warm light, raindrops on the glass, photograph"
            .into()
    });
    let seed: u64 = env("FLUX2_SEED").and_then(|s| s.parse().ok()).unwrap_or(42);
    let edits: Vec<PathBuf> = [env("FLUX2_EDIT"), env("FLUX2_EDIT2")].into_iter().flatten().map(PathBuf::from).collect();
    let repeat: usize = env("FLUX2_REPEAT").and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
    let init = env("FLUX2_INIT").map(PathBuf::from);
    let denoise: f32 = env("FLUX2_DENOISE").and_then(|s| s.parse().ok()).unwrap_or(0.6);

    flux::shared::ensure_kernels_registered();
    // Балласт: модели остаётся FLUX2_VRAM_GB.
    let _ballast = env("FLUX2_VRAM_GB").and_then(|v| v.parse::<f64>().ok()).map(|gb| {
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
    let n_ckpt = ctx.add_node(NodeKind::Flux2Checkpoint, zero);
    let n_prompt = ctx.add_node(NodeKind::TextView, zero);
    let n_enc = ctx.add_node(NodeKind::Flux2TextEncoder, zero);
    let n_smp = ctx.add_node(NodeKind::Flux2Sampler, zero);
    let n_dec = ctx.add_node(NodeKind::Flux2VaeDecode, zero);
    let n_save = ctx.add_node(NodeKind::ImageSave, zero);
    for t in [n_enc, n_smp, n_dec] {
        connect(&ctx, n_ckpt, "model", t, "model");
    }
    connect(&ctx, n_prompt, "out", n_enc, "prompt");
    connect(&ctx, n_enc, "conditioning", n_smp, "conditioning");
    connect(&ctx, n_smp, "latent", n_dec, "latent");
    connect(&ctx, n_dec, "image", n_save, "image");

    // Референсы цепочкой.
    let mut refs: Vec<NodeId> = Vec::new();
    for (i, src) in edits.iter().enumerate() {
        let n_img = ctx.add_node(NodeKind::ImageLoad, zero);
        let n_ref = ctx.add_node(NodeKind::Flux2Reference, zero);
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
        connect(&ctx, *last, "references", n_smp, "references");
    }
    // Img2img: Image → VAE Encode → Sampler.latent; размер (если задан) — во
    // вход size энкодера.
    let n_venc = init.as_ref().map(|src| {
        let n_img = ctx.add_node(NodeKind::ImageLoad, zero);
        let n_venc = ctx.add_node(NodeKind::Flux2VaeEncode, zero);
        connect(&ctx, n_ckpt, "model", n_venc, "model");
        connect(&ctx, n_img, "image", n_venc, "image");
        connect(&ctx, n_venc, "latent", n_smp, "latent");
        if let NodeRuntime::ImageLoad { path, .. } = &*node(&ctx, n_img).runtime.lock().unwrap() {
            path.set(Some(src.clone()));
        }
        if let NodeRuntime::Flux2Sampler { denoise: d, .. } = &*node(&ctx, n_smp).runtime.lock().unwrap() {
            d.set(denoise);
        }
        n_venc
    });
    let default_size = if refs.is_empty() && n_venc.is_none() { Some((1024, 1024)) } else { None };
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

    if let NodeRuntime::Flux2Checkpoint { model_path: mp, quant_idx, memory_mode_idx, resident, .. } =
        &*node(&ctx, n_ckpt).runtime.lock().unwrap()
    {
        mp.set(Some(model_path.clone()));
        if let Some(q) = env("FLUX2_QUANT") {
            quant_idx.set(match q.as_str() {
                "nvfp4" => 0,
                "dense" => 2,
                _ => 1,
            });
        }
        if let Some(m) = env("FLUX2_MEMORY_MODE") {
            memory_mode_idx.set(match m.as_str() {
                "resident" => 1,
                "block_offload" => 2,
                _ => 0,
            });
        }
        resident.set(env("FLUX2_RESIDENT").is_some_and(|v| v == "1"));
    }
    if let NodeRuntime::TextView { output_text, .. } = &*node(&ctx, n_prompt).runtime.lock().unwrap() {
        output_text.set(prompt.clone());
    }
    if let NodeRuntime::Flux2Sampler { steps: st, seed: sd, .. } = &*node(&ctx, n_smp).runtime.lock().unwrap() {
        st.set(steps);
        sd.set(seed);
    }
    if let NodeRuntime::ImageSave { path, .. } = &*node(&ctx, n_save).runtime.lock().unwrap() {
        path.set(Some(out.clone()));
    }

    eprintln!(
        "flux2_smoke: {} | {} | шаги {} (0 — по модели) seed {seed} | референсов {} | img2img {}",
        model_path.display(),
        size.map(|(w, h)| format!("{w}×{h}")).unwrap_or_else(|| "размер с референса/картинки".into()),
        steps,
        refs.len(),
        init.as_ref().map(|p| format!("{} denoise {denoise}", p.display())).unwrap_or_else(|| "нет".into())
    );
    for round in 1..=repeat {
        eprintln!("— прогон {round}/{repeat}");
        let enc = run_node(&ctx, n_enc, "text-encoder", flux2::text_encoder::on_run, &mut min_free)?;
        eprintln!("    {}", status(&ctx, n_enc));
        for (i, r) in refs.iter().enumerate() {
            run_node(&ctx, *r, &format!("reference-{}", i + 1), flux2::reference::on_run, &mut min_free)?;
            eprintln!("    {}", status(&ctx, *r));
        }
        if let Some(v) = n_venc {
            run_node(&ctx, v, "vae-encode", flux2::vae::encode_on_run, &mut min_free)?;
        }
        let smp = run_node(&ctx, n_smp, "sampler", flux2::sampler::on_run, &mut min_free)?;
        eprintln!("    {}", status(&ctx, n_smp));
        run_node(&ctx, n_dec, "vae-decode", flux2::vae::on_run, &mut min_free)?;
        run_node(&ctx, n_save, "image-save", image::save_on_run, &mut min_free)?;
        eprintln!("  итого: энкодер {enc:.1}s, сэмплер {smp:.1}s");
        if let NodeRuntime::Flux2Sampler { seed: sd, .. } = &*node(&ctx, n_smp).runtime.lock().unwrap() {
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
        eprintln!("flux2_smoke: ошибка: {e}");
        std::process::exit(1);
    }
}
