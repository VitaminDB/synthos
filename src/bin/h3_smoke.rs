use std::path::PathBuf;
use std::time::{Duration, Instant};

use syngui::prelude::RwSignal;
use synthos::pages::node_editor::eval;
use synthos::pages::node_editor::nodes::minimax_h3;
use synthos::pages::node_editor::state::NodeEditorCtx;
use synthos::pages::node_editor::types::{Connection, NodeId, NodeInstance, NodeKind, NodeRuntime};

fn drain() {
    syngui::async_runtime::drain_main_thread_callbacks();
}

fn refresh(ctx: &NodeEditorCtx) {
    let nodes = ctx.nodes.get_untracked();
    let conns = ctx.connections.get_untracked();
    let map = eval::evaluate_graph(&nodes, &conns, false);
    ctx.values.set(map);
}

fn node(ctx: &NodeEditorCtx, id: NodeId) -> NodeInstance {
    ctx.nodes
        .get_untracked()
        .iter()
        .find(|n| n.id == id)
        .cloned()
        .expect("нода найдена")
}

fn connect(ctx: &NodeEditorCtx, from: NodeId, fp: &'static str, to: NodeId, tp: &'static str) {
    ctx.connections.update(|c| {
        c.push(Connection { from_node: from, from_port: fp, to_node: to, to_port: tp });
    });
}

fn wait_done(
    name: &str,
    running: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    progress: Option<RwSignal<f32>>,
) -> std::result::Result<(), String> {
    let t0 = Instant::now();
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
            if (pct - last).abs() > 0.009 {
                eprintln!("  [{name}] {:.0}% ({:.1}s)", pct * 100.0, t0.elapsed().as_secs_f32());
                last = pct;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    drain();
    if let Some(e) = error.get_untracked() {
        return Err(format!("{name}: {e}"));
    }
    eprintln!("  [{name}] готово за {:.1}s", t0.elapsed().as_secs_f32());
    Ok(())
}

fn signals_encoder(n: &NodeInstance) -> (RwSignal<bool>, RwSignal<Option<String>>) {
    match &*n.runtime.lock().unwrap() {
        NodeRuntime::H3TextEncoder { running, error, .. } => (*running, *error),
        _ => panic!("ожидался H3TextEncoder"),
    }
}

fn signals_sampler(
    n: &NodeInstance,
) -> (RwSignal<bool>, RwSignal<Option<String>>, RwSignal<f32>) {
    match &*n.runtime.lock().unwrap() {
        NodeRuntime::H3Sampler { running, error, progress_pct, .. } => {
            (*running, *error, *progress_pct)
        }
        _ => panic!("ожидался H3Sampler"),
    }
}

fn signals_vae(n: &NodeInstance) -> (RwSignal<bool>, RwSignal<Option<String>>) {
    match &*n.runtime.lock().unwrap() {
        NodeRuntime::H3VaeDecode { running, error, .. } => (*running, *error),
        _ => panic!("ожидался H3VaeDecode"),
    }
}

fn signals_audio(n: &NodeInstance) -> (RwSignal<bool>, RwSignal<Option<String>>) {
    match &*n.runtime.lock().unwrap() {
        NodeRuntime::H3AudioDecode { running, error, .. } => (*running, *error),
        _ => panic!("ожидался H3AudioDecode"),
    }
}

fn signals_save(n: &NodeInstance) -> (RwSignal<bool>, RwSignal<Option<String>>) {
    match &*n.runtime.lock().unwrap() {
        NodeRuntime::H3VideoSave { running, error, .. } => (*running, *error),
        _ => panic!("ожидался H3VideoSave"),
    }
}

fn run() -> std::result::Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        return Err(format!(
            "использование: {} <model_dir> <out.mp4> [width height duration_sec steps] \
             (env: H3_LORA=<turbo.safetensors>, H3_PROMPT=..., H3_IMAGE=<first_frame>)",
            args[0]
        ));
    }
    let model_dir = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let width: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(640);
    let height: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(384);
    let dur: f32 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(2.0);
    let steps: u32 = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(6);
    if !model_dir.exists() {
        return Err(format!("нет каталога: {}", model_dir.display()));
    }
    let prompt = std::env::var("H3_PROMPT").unwrap_or_else(|_| {
        "a calm sunlit room, dust motes drifting in the light, soft ambient hum".into()
    });
    let lora = std::env::var("H3_LORA").ok().filter(|s| !s.is_empty()).map(PathBuf::from);
    let image = std::env::var("H3_IMAGE").ok().filter(|s| !s.is_empty()).map(PathBuf::from);

    minimax_h3::shared::ensure_kernels_registered();
    if std::env::var("H3_PROF").is_ok_and(|v| v != "0") {
        synaptix_video_minimax_h3::runtime::set_h3_prof(true);
        synaptix_video_minimax_h3::runtime::set_h3_vae_prof(true);
        synaptix_video_minimax_h3::runtime::set_h3_blk_prof(true);
        synaptix_video_minimax_h3::runtime::set_h3_adaln_prof(true);
        synaptix_video_minimax_h3::runtime::set_h3_attn_prof(true);
        synaptix_video_minimax_h3::runtime::set_h3_mlp_prof(true);
    }
    if let Some(b) = std::env::var("H3_PROF_BLOCK").ok().and_then(|v| v.parse::<usize>().ok()) {
        synaptix_video_minimax_h3::runtime::set_prof_block(b);
    }
    if let Some(n) = std::env::var("H3_NBLOCKS").ok().and_then(|v| v.parse::<usize>().ok()) {
        synaptix_video_minimax_h3::runtime::set_nblocks_cap(Some(n));
    }

    let ctx = NodeEditorCtx::new();
    let zero = syngui::core::Point::new(0.0, 0.0);
    let n_ckpt = ctx.add_node(NodeKind::H3Checkpoint, zero);
    let n_prompt = ctx.add_node(NodeKind::TextView, zero);
    let n_enc = ctx.add_node(NodeKind::H3TextEncoder, zero);
    let n_lat = ctx.add_node(NodeKind::H3EmptyLatentAv, zero);
    let n_smp = ctx.add_node(NodeKind::H3Sampler, zero);
    let n_vae = ctx.add_node(NodeKind::H3VaeDecode, zero);
    let n_aud = ctx.add_node(NodeKind::H3AudioDecode, zero);
    let n_save = ctx.add_node(NodeKind::H3VideoSave, zero);
    let n_kf = image.as_ref().map(|_| ctx.add_node(NodeKind::H3Keyframe, zero));

    for t in [n_enc, n_smp, n_vae, n_aud] {
        connect(&ctx, n_ckpt, "model", t, "model");
    }
    connect(&ctx, n_prompt, "out", n_enc, "prompt");
    connect(&ctx, n_enc, "conditioning", n_smp, "conditioning");
    connect(&ctx, n_lat, "av_latent", n_smp, "av_latent");
    connect(&ctx, n_smp, "video_latent", n_vae, "video_latent");
    connect(&ctx, n_smp, "audio_latent", n_aud, "audio_latent");
    connect(&ctx, n_vae, "frames", n_save, "frames");
    connect(&ctx, n_aud, "audio", n_save, "audio");
    if let Some(kf) = n_kf {
        connect(&ctx, kf, "keyframe", n_enc, "keyframe");
        connect(&ctx, kf, "keyframe", n_smp, "keyframe");
    }

    match &*node(&ctx, n_ckpt).runtime.lock().unwrap() {
        NodeRuntime::H3Checkpoint { model_dir: md, lora_path, quant_dit_idx, .. } => {
            md.set(Some(model_dir.clone()));
            lora_path.set(lora.clone());
            if let Ok(q) = std::env::var("H3_QUANT_DIT") {
                quant_dit_idx.set(match q.as_str() {
                    "mxfp8" => 1,
                    "dense" => 2,
                    _ => 0,
                });
            }
        }
        _ => return Err("H3Checkpoint runtime".into()),
    }
    match &*node(&ctx, n_prompt).runtime.lock().unwrap() {
        NodeRuntime::TextView { output_text, .. } => output_text.set(prompt.clone()),
        _ => return Err("TextView runtime".into()),
    }
    match &*node(&ctx, n_lat).runtime.lock().unwrap() {
        NodeRuntime::H3EmptyLatentAv { width: w, height: h, duration_seconds } => {
            w.set(width);
            h.set(height);
            duration_seconds.set(dur);
        }
        _ => return Err("H3EmptyLatentAv runtime".into()),
    }
    match &*node(&ctx, n_smp).runtime.lock().unwrap() {
        NodeRuntime::H3Sampler { steps: s, cfg_scale, .. } => {
            s.set(steps);
            cfg_scale.set(if lora.is_some() { 1.0 } else { 5.0 });
        }
        _ => return Err("H3Sampler runtime".into()),
    }
    match &*node(&ctx, n_save).runtime.lock().unwrap() {
        NodeRuntime::H3VideoSave { path, .. } => path.set(Some(out.clone())),
        _ => return Err("H3VideoSave runtime".into()),
    }
    if let (Some(kf), Some(img)) = (n_kf, image.as_ref()) {
        match &*node(&ctx, kf).runtime.lock().unwrap() {
            NodeRuntime::H3Keyframe { path, .. } => path.set(Some(img.clone())),
            _ => return Err("H3Keyframe runtime".into()),
        }
        refresh(&ctx);
        minimax_h3::latent::keyframe_on_run(&node(&ctx, kf), &ctx);
        drain();
    }

    refresh(&ctx);
    eprintln!("[h3-smoke] {width}x{height}, {dur} с, {steps} шагов, LoRA: {}", lora.is_some());

    let n = node(&ctx, n_enc);
    minimax_h3::text_encoder::on_run(&n, &ctx);
    let (r, e) = signals_encoder(&n);
    wait_done("text-encoder", r, e, None)?;
    refresh(&ctx);

    let n = node(&ctx, n_smp);
    minimax_h3::sampler::on_run(&n, &ctx);
    let (r, e, p) = signals_sampler(&n);
    wait_done("sampler", r, e, Some(p))?;
    refresh(&ctx);

    let n = node(&ctx, n_vae);
    minimax_h3::decode::vae_on_run(&n, &ctx);
    let (r, e) = signals_vae(&n);
    wait_done("vae-decode", r, e, None)?;
    refresh(&ctx);

    let n = node(&ctx, n_aud);
    minimax_h3::decode::audio_on_run(&n, &ctx);
    let (r, e) = signals_audio(&n);
    wait_done("audio-decode", r, e, None)?;
    refresh(&ctx);

    let n = node(&ctx, n_save);
    minimax_h3::save::on_run(&n, &ctx);
    let (r, e) = signals_save(&n);
    wait_done("video-save", r, e, None)?;

    let meta = std::fs::metadata(&out).map_err(|x| format!("{}: {x}", out.display()))?;
    if meta.len() == 0 {
        return Err(format!("{} пустой", out.display()));
    }
    eprintln!("[h3-smoke] {} — {:.1} МБ", out.display(), meta.len() as f64 / 1e6);
    Ok(())
}

fn main() {
    syngui::signal::init_main_thread();
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
