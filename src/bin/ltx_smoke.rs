//! Headless smoke-раннер нодного LTX-пайплайна (без GUI): собирает граф
//! «LTX: Text to Video» программно, прогоняет sequencer-последовательность
//! воркеров (text encoder → NAG → stage1 → upscale → stage2 → VAE/audio
//! decode → mp4) и проверяет выходной файл.
//!
//! Кросс-поточные `RwSignal::set` воркеров применяются дренажём
//! `syngui::async_runtime::drain_main_thread_callbacks` (без окна канал
//! main-thread колбэков никто не выкачивает).
//!
//! Запуск: `ltx_smoke <ckpt.safetensors> <gemma_dir> <upscaler.safetensors>
//! <out.mp4> [width height duration_sec]`

use std::path::PathBuf;
use std::time::{Duration, Instant};

use syngui::prelude::RwSignal;
use synthos::pages::node_editor::eval;
use synthos::pages::node_editor::state::NodeEditorCtx;
use synthos::pages::node_editor::nodes::ltx;
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

fn node<'a>(ctx: &'a NodeEditorCtx, id: NodeId) -> NodeInstance {
    ctx.nodes
        .get_untracked()
        .iter()
        .find(|n| n.id == id)
        .cloned()
        .expect("нода найдена")
}

fn wait_done(
    name: &str,
    running: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    progress: Option<RwSignal<f32>>,
) -> Result<(), String> {
    let t0 = Instant::now();
    let mut last_pct = -1.0f32;
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
            if (pct - last_pct).abs() > 0.009 {
                eprintln!("  [{name}] {:.0}% ({:.1}s)", pct * 100.0, t0.elapsed().as_secs_f32());
                last_pct = pct;
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

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        return Err(format!(
            "использование: {} <ckpt.safetensors> <gemma_dir> <upscaler.safetensors> <out.mp4> [width height duration_sec]",
            args[0]
        ));
    }
    let ckpt = PathBuf::from(&args[1]);
    let gemma = PathBuf::from(&args[2]);
    let upscaler = PathBuf::from(&args[3]);
    let out = args[4].clone();
    let width: u32 = args.get(5).map(|s| s.parse().unwrap_or(256)).unwrap_or(256);
    let height: u32 = args.get(6).map(|s| s.parse().unwrap_or(256)).unwrap_or(256);
    let dur: f32 = args.get(7).map(|s| s.parse().unwrap_or(2.0)).unwrap_or(2.0);
    for p in [&ckpt, &gemma, &upscaler] {
        if !p.exists() {
            return Err(format!("нет файла/каталога: {}", p.display()));
        }
    }

    // retake-режим: SYN_SMOKE_RETAKE=<video> строит граф VideoInput→Retake→Decode.
    if let Ok(vid) = std::env::var("SYN_SMOKE_RETAKE") {
        return run_retake(&ckpt, &gemma, &vid, &out, width, height, dur);
    }
    // ic-lora: SYN_SMOKE_ICLORA=<ref_video> + SYN_SMOKE_LORA=<ic-lora.safetensors>.
    if let Ok(refv) = std::env::var("SYN_SMOKE_ICLORA") {
        let lora = std::env::var("SYN_SMOKE_LORA").map_err(|_| "SYN_SMOKE_LORA не задан".to_string())?;
        return run_ic_lora(&ckpt, &gemma, &lora, &refv, &out, width, height, dur);
    }
    // lipdub: SYN_SMOKE_LIPDUB=<ref_video> + SYN_SMOKE_AUDIO=<wav> + SYN_SMOKE_LORA.
    if let Ok(refv) = std::env::var("SYN_SMOKE_LIPDUB") {
        let audio = std::env::var("SYN_SMOKE_AUDIO").map_err(|_| "SYN_SMOKE_AUDIO не задан".to_string())?;
        let lora = std::env::var("SYN_SMOKE_LORA").ok();
        return run_lipdub(&ckpt, &gemma, &upscaler, lora.as_deref(), &refv, &audio, &out, width, height, dur);
    }
    // a2v: SYN_SMOKE_A2V=<audio>.
    if let Ok(audio) = std::env::var("SYN_SMOKE_A2V") {
        return run_a2v(&ckpt, &gemma, &upscaler, &audio, &out, width, height, dur);
    }

    let ctx = NodeEditorCtx::new();
    let zero = syngui::core::Point::new(0.0, 0.0);
    let n_ckpt = ctx.add_node(NodeKind::LtxCheckpoint, zero);
    let n_enc = ctx.add_node(NodeKind::LtxTextEncoder, zero);
    let n_nag = ctx.add_node(NodeKind::LtxNagPrompt, zero);
    let n_s1 = ctx.add_node(NodeKind::LtxSamplerStage1, zero);
    let n_up = ctx.add_node(NodeKind::LtxUpscale, zero);
    let n_s2 = ctx.add_node(NodeKind::LtxSamplerStage2, zero);
    let n_vae = ctx.add_node(NodeKind::LtxVaeDecode, zero);
    let n_aud = ctx.add_node(NodeKind::LtxAudioDecode, zero);
    let n_save = ctx.add_node(NodeKind::LtxVideoSave, zero);
    // i2v: SYN_SMOKE_IMAGE=<path> → подключить LTX Image к Sampler.image_cond.
    let smoke_image = std::env::var("SYN_SMOKE_IMAGE").ok().filter(|s| !s.is_empty());
    let n_img = smoke_image.as_ref().map(|_| ctx.add_node(NodeKind::LtxImage, zero));

    {
        let inst = node(&ctx, n_ckpt);
        let g = inst.runtime.lock().map_err(|_| "runtime lock")?;
        if let NodeRuntime::LtxCheckpoint {
            model_path,
            gemma_dir,
            upscaler_path,
            quant_dit_idx,
            ..
        } = &*g
        {
            model_path.set(Some(ckpt));
            gemma_dir.set(Some(gemma));
            upscaler_path.set(Some(upscaler));
            if let Ok(q) = std::env::var("SYN_SMOKE_QUANT_DIT") {
                if let Some(i) = ltx::QUANT_DIT_OPTIONS
                    .iter()
                    .position(|o| o.starts_with(q.as_str()))
                {
                    quant_dit_idx.set(i);
                }
            }
        }
    }
    {
        let inst = node(&ctx, n_enc);
        let g = inst.runtime.lock().map_err(|_| "runtime lock")?;
        if let NodeRuntime::LtxTextEncoder { prompt_field, .. } = &*g {
            prompt_field.set(
                "a woman in a cozy cafe, talking warmly to the camera, soft daylight, \
                 gentle background chatter"
                    .into(),
            );
        }
    }
    {
        let inst = node(&ctx, n_s1);
        let g = inst.runtime.lock().map_err(|_| "runtime lock")?;
        if let NodeRuntime::LtxSamplerStage1 {
            width: w,
            height: h,
            duration_seconds,
            ..
        } = &*g
        {
            w.set(width);
            h.set(height);
            duration_seconds.set(dur);
        }
    }
    if std::env::var("SYN_SMOKE_NO_NAG").as_deref() == Ok("1") {
        let inst = node(&ctx, n_nag);
        let g = inst.runtime.lock().map_err(|_| "runtime lock")?;
        if let NodeRuntime::LtxNagPrompt { prompt_field, .. } = &*g {
            prompt_field.set(String::new());
        }
    }
    {
        let inst = node(&ctx, n_save);
        let g = inst.runtime.lock().map_err(|_| "runtime lock")?;
        if let NodeRuntime::LtxVideoSave { path, .. } = &*g {
            path.set(out.clone());
        }
    }
    if let (Some(n_img), Some(img_path)) = (n_img, &smoke_image) {
        let inst = node(&ctx, n_img);
        let g = inst.runtime.lock().map_err(|_| "runtime lock")?;
        if let NodeRuntime::LtxImage { image_path, frame_idx, .. } = &*g {
            image_path.set(Some(PathBuf::from(img_path)));
            // SYN_SMOKE_FRAME=N → keyframe на пиксель-кадре N (0 = i2v replace).
            if let Ok(f) = std::env::var("SYN_SMOKE_FRAME") {
                if let Ok(n) = f.parse::<u32>() {
                    frame_idx.set(n);
                }
            }
        }
    }

    let conn = |from: NodeId, fp: &'static str, to: NodeId, tp: &'static str| Connection {
        from_node: from,
        from_port: fp,
        to_node: to,
        to_port: tp,
    };
    let mut conns = vec![
        conn(n_ckpt, "model", n_enc, "model"),
        conn(n_ckpt, "model", n_nag, "model"),
        conn(n_ckpt, "model", n_s1, "model"),
        conn(n_ckpt, "model", n_up, "model"),
        conn(n_ckpt, "model", n_s2, "model"),
        conn(n_ckpt, "model", n_vae, "model"),
        conn(n_ckpt, "model", n_aud, "model"),
        conn(n_enc, "video_encoding", n_s1, "video_encoding"),
        conn(n_enc, "audio_encoding", n_s1, "audio_encoding"),
        conn(n_nag, "nag", n_s1, "nag"),
        conn(n_s1, "video_latent", n_up, "video_latent"),
        conn(n_up, "video_latent", n_s2, "video_latent"),
        conn(n_s1, "audio_tokens", n_s2, "audio_tokens"),
        conn(n_enc, "video_encoding", n_s2, "video_encoding"),
        conn(n_enc, "audio_encoding", n_s2, "audio_encoding"),
        conn(n_s2, "video_latent", n_vae, "video_latent"),
        conn(n_s2, "audio_tokens", n_aud, "audio_tokens"),
        conn(n_vae, "frames", n_save, "frames"),
        conn(n_aud, "audio", n_save, "audio"),
    ];
    if let Some(n_img) = n_img {
        conns.push(conn(n_img, "image_cond", n_s1, "image_cond"));
        conns.push(conn(n_img, "image_cond", n_s2, "image_cond"));
    }
    ctx.connections.set(conns);

    macro_rules! step {
        ($name:literal, $id:expr, $start:path, $variant:path { running: $r:ident, error: $e:ident $(, progress_pct: $p:ident)? }) => {{
            refresh(&ctx);
            let inst = node(&ctx, $id);
            $start(&inst, &ctx);
            let (running, error, progress) = {
                let g = inst.runtime.lock().map_err(|_| "runtime lock")?;
                match &*g {
                    $variant { $r, $e, .. } => {
                        #[allow(unused_mut, unused_assignments)]
                        let mut prog: Option<RwSignal<f32>> = None;
                        $(
                            prog = match &*g {
                                $variant { $p, .. } => Some(*$p),
                                _ => None,
                            };
                        )?
                        (*$r, *$e, prog)
                    }
                    _ => return Err(concat!($name, ": неожиданный runtime").into()),
                }
            };
            wait_done($name, running, error, progress)?;
            if let Ok((free, _)) = synaptix_core::device::cuda::mem_info(0) {
                eprintln!("  [{}] свободно VRAM после стадии: {:.2} ГБ", $name, free as f64 / 1e9);
            }
        }};
    }

    eprintln!("[ltx_smoke] {}×{} {:.1}s → {}", width, height, dur, out);
    step!("text-encoder", n_enc, ltx::text_encoder::start, NodeRuntime::LtxTextEncoder { running: running, error: error, progress_pct: progress_pct });
    if let Ok(dir) = std::env::var("SYN_SMOKE_CTX_SAVE") {
        let inst = node(&ctx, n_enc);
        let g = inst.runtime.lock().map_err(|_| "runtime lock")?;
        if let NodeRuntime::LtxTextEncoder { v_out, a_out, .. } = &*g {
            let dump = |t: &synaptix_core::tensor::Tensor, name: &str| -> Result<(), String> {
                let n: usize = t.dims().iter().product();
                let v = t
                    .to_device(synaptix_core::device::Device::Cpu)
                    .and_then(|t| t.to_dtype(synaptix_core::dtype::DType::F32))
                    .and_then(|t| t.reshape(vec![n]))
                    .and_then(|t| t.to_vec1::<f32>())
                    .map_err(|e| format!("dump {name}: {e}"))?;
                let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
                let dims: Vec<String> = t.dims().iter().map(|d| d.to_string()).collect();
                let p = format!("{dir}/{name}.{}.f32", dims.join("x"));
                std::fs::write(&p, bytes).map_err(|e| format!("write {p}: {e}"))?;
                eprintln!("[SMOKE_CTX_SAVE] {p}");
                Ok(())
            };
            if let Ok(b) = v_out.lock() {
                if let Some(t) = b.as_ref() {
                    dump(t, "v_enc")?;
                }
            }
            if let Ok(b) = a_out.lock() {
                if let Some(t) = b.as_ref() {
                    dump(t, "a_enc")?;
                }
            }
        }
    }
    step!("nag", n_nag, ltx::nag::start, NodeRuntime::LtxNagPrompt { running: running, error: error });
    step!("sampler-stage1", n_s1, ltx::sampler_stage1::start, NodeRuntime::LtxSamplerStage1 { running: running, error: error, progress_pct: progress_pct });
    step!("upscale", n_up, ltx::upscale::start, NodeRuntime::LtxUpscale { running: running, error: error });
    step!("sampler-stage2", n_s2, ltx::sampler_stage2::start, NodeRuntime::LtxSamplerStage2 { running: running, error: error, progress_pct: progress_pct });
    step!("vae-decode", n_vae, ltx::vae_decode::start, NodeRuntime::LtxVaeDecode { running: running, error: error, progress_pct: progress_pct });
    step!("audio-decode", n_aud, ltx::audio_decode::start, NodeRuntime::LtxAudioDecode { running: running, error: error });
    step!("video-save", n_save, ltx::video_save::start, NodeRuntime::LtxVideoSave { running: running, error: error, progress_pct: progress_pct });

    let meta = std::fs::metadata(&out).map_err(|e| format!("выходной файл: {e}"))?;
    if meta.len() == 0 {
        return Err("выходной mp4 пуст".into());
    }
    refresh(&ctx);
    {
        let inst = node(&ctx, n_vae);
        let g = inst.runtime.lock().map_err(|_| "runtime lock")?;
        if let NodeRuntime::LtxVaeDecode { frames_out, .. } = &*g {
            let fr = frames_out
                .lock()
                .ok()
                .and_then(|b| b.clone())
                .ok_or("кадры не опубликованы")?;
            eprintln!(
                "[ltx_smoke] OK: {} кадров {}×{} @ {:.0}fps, mp4 {} байт",
                fr.frames.len(),
                fr.width,
                fr.height,
                fr.fps,
                meta.len()
            );
        }
    }
    Ok(())
}

/// retake-граф: Checkpoint + TextEncoder + VideoInput → Retake → VAE/Audio
/// Decode → Save. SYN_SMOKE_RETAKE_START/END — границы региона (сек).
fn run_retake(
    ckpt: &PathBuf,
    gemma: &PathBuf,
    video: &str,
    out: &str,
    width: u32,
    height: u32,
    dur: f32,
) -> Result<(), String> {
    let ctx = NodeEditorCtx::new();
    let zero = syngui::core::Point::new(0.0, 0.0);
    let n_ckpt = ctx.add_node(NodeKind::LtxCheckpoint, zero);
    let n_enc = ctx.add_node(NodeKind::LtxTextEncoder, zero);
    let n_vin = ctx.add_node(NodeKind::LtxVideoInput, zero);
    let n_rt = ctx.add_node(NodeKind::LtxRetake, zero);
    let n_vae = ctx.add_node(NodeKind::LtxVaeDecode, zero);
    let n_aud = ctx.add_node(NodeKind::LtxAudioDecode, zero);
    let n_save = ctx.add_node(NodeKind::LtxVideoSave, zero);

    {
        let inst = node(&ctx, n_ckpt);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxCheckpoint { model_path, gemma_dir, .. } = &*g {
            model_path.set(Some(ckpt.clone()));
            gemma_dir.set(Some(gemma.clone()));
        }
    }
    {
        let inst = node(&ctx, n_enc);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxTextEncoder { prompt_field, .. } = &*g {
            prompt_field.set("same scene, smooth natural motion".into());
        }
    }
    {
        let inst = node(&ctx, n_vin);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxVideoInput { video_path } = &*g {
            video_path.set(Some(PathBuf::from(video)));
        }
    }
    let rs: f32 = std::env::var("SYN_SMOKE_RETAKE_START").ok().and_then(|s| s.parse().ok()).unwrap_or(0.5);
    let re: f32 = std::env::var("SYN_SMOKE_RETAKE_END").ok().and_then(|s| s.parse().ok()).unwrap_or(1.5);
    {
        let inst = node(&ctx, n_rt);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxRetake { width: w, height: h, duration_seconds, retake_start, retake_end, .. } = &*g {
            w.set(width);
            h.set(height);
            duration_seconds.set(dur);
            retake_start.set(rs);
            retake_end.set(re);
        }
    }
    {
        let inst = node(&ctx, n_save);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxVideoSave { path, .. } = &*g {
            path.set(out.to_string());
        }
    }

    let conn = |from: NodeId, fp: &'static str, to: NodeId, tp: &'static str| Connection {
        from_node: from, from_port: fp, to_node: to, to_port: tp,
    };
    ctx.connections.set(vec![
        conn(n_ckpt, "model", n_enc, "model"),
        conn(n_ckpt, "model", n_rt, "model"),
        conn(n_ckpt, "model", n_vae, "model"),
        conn(n_ckpt, "model", n_aud, "model"),
        conn(n_enc, "video_encoding", n_rt, "video_encoding"),
        conn(n_enc, "audio_encoding", n_rt, "audio_encoding"),
        conn(n_vin, "video", n_rt, "video"),
        conn(n_rt, "video_latent", n_vae, "video_latent"),
        conn(n_rt, "audio_tokens", n_aud, "audio_tokens"),
        conn(n_vae, "frames", n_save, "frames"),
        conn(n_aud, "audio", n_save, "audio"),
    ]);

    eprintln!("[ltx_smoke] retake {}×{} {:.1}s регион [{rs},{re}] → {out}", width, height, dur);
    let run_step = |name: &str, id: NodeId, start: fn(&NodeInstance, &NodeEditorCtx)| -> Result<(), String> {
        refresh(&ctx);
        let inst = node(&ctx, id);
        start(&inst, &ctx);
        let (running, error, progress) = {
            let g = inst.runtime.lock().map_err(|_| "lock")?;
            match &*g {
                NodeRuntime::LtxTextEncoder { running, error, progress_pct, .. }
                | NodeRuntime::LtxRetake { running, error, progress_pct, .. }
                | NodeRuntime::LtxVaeDecode { running, error, progress_pct, .. }
                | NodeRuntime::LtxVideoSave { running, error, progress_pct, .. } => {
                    (*running, *error, Some(*progress_pct))
                }
                NodeRuntime::LtxAudioDecode { running, error, .. } => (*running, *error, None),
                _ => return Err(format!("{name}: неожиданный runtime")),
            }
        };
        wait_done(name, running, error, progress)
    };
    run_step("text-encoder", n_enc, ltx::text_encoder::start)?;
    run_step("retake", n_rt, ltx::retake::start)?;
    run_step("vae-decode", n_vae, ltx::vae_decode::start)?;
    run_step("audio-decode", n_aud, ltx::audio_decode::start)?;
    run_step("video-save", n_save, ltx::video_save::start)?;

    let meta = std::fs::metadata(out).map_err(|e| format!("выходной файл: {e}"))?;
    if meta.len() == 0 {
        return Err("выходной mp4 пуст".into());
    }
    eprintln!("[ltx_smoke] retake OK: mp4 {} байт", meta.len());
    Ok(())
}

/// ic-lora-граф: Checkpoint(+IC-LoRA) + TextEncoder + VideoInput(ref) → IC-LoRA
/// → VAE/Audio Decode → Save.
fn run_ic_lora(
    ckpt: &PathBuf,
    gemma: &PathBuf,
    lora: &str,
    ref_video: &str,
    out: &str,
    width: u32,
    height: u32,
    dur: f32,
) -> Result<(), String> {
    let ctx = NodeEditorCtx::new();
    let zero = syngui::core::Point::new(0.0, 0.0);
    let n_ckpt = ctx.add_node(NodeKind::LtxCheckpoint, zero);
    let n_enc = ctx.add_node(NodeKind::LtxTextEncoder, zero);
    let n_vin = ctx.add_node(NodeKind::LtxVideoInput, zero);
    let n_ic = ctx.add_node(NodeKind::LtxIcLora, zero);
    let n_vae = ctx.add_node(NodeKind::LtxVaeDecode, zero);
    let n_aud = ctx.add_node(NodeKind::LtxAudioDecode, zero);
    let n_save = ctx.add_node(NodeKind::LtxVideoSave, zero);

    {
        let inst = node(&ctx, n_ckpt);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxCheckpoint { model_path, gemma_dir, lora_path, depth_model_path, .. } = &*g {
            model_path.set(Some(ckpt.clone()));
            gemma_dir.set(Some(gemma.clone()));
            lora_path.set(Some(PathBuf::from(lora)));
            // SYN_SMOKE_DEPTH=<dir> — каталог Depth Anything V2 для control=depth.
            if let Ok(d) = std::env::var("SYN_SMOKE_DEPTH") {
                depth_model_path.set(Some(PathBuf::from(d)));
            }
        }
    }
    {
        let inst = node(&ctx, n_enc);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxTextEncoder { prompt_field, .. } = &*g {
            prompt_field.set("a vibrant scene, cinematic lighting".into());
        }
    }
    {
        let inst = node(&ctx, n_vin);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxVideoInput { video_path } = &*g {
            video_path.set(Some(PathBuf::from(ref_video)));
        }
    }
    {
        let inst = node(&ctx, n_ic);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxIcLora { width: w, height: h, duration_seconds, control_idx, .. } = &*g {
            w.set(width);
            h.set(height);
            duration_seconds.set(dur);
            // SYN_SMOKE_CONTROL=canny|depth (depth — с SYN_SMOKE_DEPTH на чекпойнте).
            match std::env::var("SYN_SMOKE_CONTROL").as_deref() {
                Ok("canny") => control_idx.set(1),
                Ok("depth") => control_idx.set(2),
                _ => {}
            }
        }
    }
    {
        let inst = node(&ctx, n_save);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxVideoSave { path, .. } = &*g {
            path.set(out.to_string());
        }
    }

    let conn = |from: NodeId, fp: &'static str, to: NodeId, tp: &'static str| Connection {
        from_node: from, from_port: fp, to_node: to, to_port: tp,
    };
    ctx.connections.set(vec![
        conn(n_ckpt, "model", n_enc, "model"),
        conn(n_ckpt, "model", n_ic, "model"),
        conn(n_ckpt, "model", n_vae, "model"),
        conn(n_ckpt, "model", n_aud, "model"),
        conn(n_enc, "video_encoding", n_ic, "video_encoding"),
        conn(n_enc, "audio_encoding", n_ic, "audio_encoding"),
        conn(n_vin, "video", n_ic, "ref_video"),
        conn(n_ic, "video_latent", n_vae, "video_latent"),
        conn(n_ic, "audio_tokens", n_aud, "audio_tokens"),
        conn(n_vae, "frames", n_save, "frames"),
        conn(n_aud, "audio", n_save, "audio"),
    ]);

    eprintln!("[ltx_smoke] ic-lora {}×{} {:.1}s → {out}", width, height, dur);
    let run_step = |name: &str, id: NodeId, start: fn(&NodeInstance, &NodeEditorCtx)| -> Result<(), String> {
        refresh(&ctx);
        let inst = node(&ctx, id);
        start(&inst, &ctx);
        let (running, error, progress) = {
            let g = inst.runtime.lock().map_err(|_| "lock")?;
            match &*g {
                NodeRuntime::LtxTextEncoder { running, error, progress_pct, .. }
                | NodeRuntime::LtxIcLora { running, error, progress_pct, .. }
                | NodeRuntime::LtxVaeDecode { running, error, progress_pct, .. }
                | NodeRuntime::LtxVideoSave { running, error, progress_pct, .. } => {
                    (*running, *error, Some(*progress_pct))
                }
                NodeRuntime::LtxAudioDecode { running, error, .. } => (*running, *error, None),
                _ => return Err(format!("{name}: неожиданный runtime")),
            }
        };
        wait_done(name, running, error, progress)
    };
    run_step("text-encoder", n_enc, ltx::text_encoder::start)?;
    run_step("ic-lora", n_ic, ltx::ic_lora::start)?;
    run_step("vae-decode", n_vae, ltx::vae_decode::start)?;
    run_step("audio-decode", n_aud, ltx::audio_decode::start)?;
    run_step("video-save", n_save, ltx::video_save::start)?;

    let meta = std::fs::metadata(out).map_err(|e| format!("выходной файл: {e}"))?;
    if meta.len() == 0 {
        return Err("выходной mp4 пуст".into());
    }
    eprintln!("[ltx_smoke] ic-lora OK: mp4 {} байт", meta.len());
    Ok(())
}

/// lipdub-граф: Checkpoint(+upscaler/lipdub-LoRA) + TextEncoder + VideoInput(ref)
/// + AudioInput(речь) → Lipdub → VAE/Audio Decode → Save.
#[allow(clippy::too_many_arguments)]
fn run_lipdub(
    ckpt: &PathBuf,
    gemma: &PathBuf,
    upscaler: &PathBuf,
    lora: Option<&str>,
    ref_video: &str,
    audio: &str,
    out: &str,
    width: u32,
    height: u32,
    dur: f32,
) -> Result<(), String> {
    let ctx = NodeEditorCtx::new();
    let zero = syngui::core::Point::new(0.0, 0.0);
    let n_ckpt = ctx.add_node(NodeKind::LtxCheckpoint, zero);
    let n_enc = ctx.add_node(NodeKind::LtxTextEncoder, zero);
    let n_vin = ctx.add_node(NodeKind::LtxVideoInput, zero);
    let n_ain = ctx.add_node(NodeKind::LtxAudioInput, zero);
    let n_lip = ctx.add_node(NodeKind::LtxLipdub, zero);
    let n_vae = ctx.add_node(NodeKind::LtxVaeDecode, zero);
    let n_aud = ctx.add_node(NodeKind::LtxAudioDecode, zero);
    let n_save = ctx.add_node(NodeKind::LtxVideoSave, zero);

    {
        let inst = node(&ctx, n_ckpt);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxCheckpoint { model_path, gemma_dir, upscaler_path, lora_path, .. } = &*g {
            model_path.set(Some(ckpt.clone()));
            gemma_dir.set(Some(gemma.clone()));
            upscaler_path.set(Some(upscaler.clone()));
            lora_path.set(lora.map(PathBuf::from));
        }
    }
    {
        let inst = node(&ctx, n_enc);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxTextEncoder { prompt_field, .. } = &*g {
            prompt_field.set("a person speaking to the camera".into());
        }
    }
    {
        let inst = node(&ctx, n_vin);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxVideoInput { video_path } = &*g {
            video_path.set(Some(PathBuf::from(ref_video)));
        }
    }
    {
        let inst = node(&ctx, n_ain);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxAudioInput { audio_path } = &*g {
            audio_path.set(Some(PathBuf::from(audio)));
        }
    }
    {
        let inst = node(&ctx, n_lip);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxLipdub { width: w, height: h, duration_seconds, .. } = &*g {
            w.set(width);
            h.set(height);
            duration_seconds.set(dur);
        }
    }
    {
        let inst = node(&ctx, n_save);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxVideoSave { path, .. } = &*g {
            path.set(out.to_string());
        }
    }

    let conn = |from: NodeId, fp: &'static str, to: NodeId, tp: &'static str| Connection {
        from_node: from, from_port: fp, to_node: to, to_port: tp,
    };
    ctx.connections.set(vec![
        conn(n_ckpt, "model", n_enc, "model"),
        conn(n_ckpt, "model", n_lip, "model"),
        conn(n_ckpt, "model", n_vae, "model"),
        conn(n_ckpt, "model", n_aud, "model"),
        conn(n_enc, "video_encoding", n_lip, "video_encoding"),
        conn(n_enc, "audio_encoding", n_lip, "audio_encoding"),
        conn(n_vin, "video", n_lip, "ref_video"),
        conn(n_ain, "audio", n_lip, "audio"),
        conn(n_lip, "video_latent", n_vae, "video_latent"),
        conn(n_lip, "audio_tokens", n_aud, "audio_tokens"),
        conn(n_vae, "frames", n_save, "frames"),
        conn(n_aud, "audio", n_save, "audio"),
    ]);

    eprintln!("[ltx_smoke] lipdub {}×{} {:.1}s → {out}", width, height, dur);
    let run_step = |name: &str, id: NodeId, start: fn(&NodeInstance, &NodeEditorCtx)| -> Result<(), String> {
        refresh(&ctx);
        let inst = node(&ctx, id);
        start(&inst, &ctx);
        let (running, error, progress) = {
            let g = inst.runtime.lock().map_err(|_| "lock")?;
            match &*g {
                NodeRuntime::LtxTextEncoder { running, error, progress_pct, .. }
                | NodeRuntime::LtxLipdub { running, error, progress_pct, .. }
                | NodeRuntime::LtxVaeDecode { running, error, progress_pct, .. }
                | NodeRuntime::LtxVideoSave { running, error, progress_pct, .. } => {
                    (*running, *error, Some(*progress_pct))
                }
                NodeRuntime::LtxAudioDecode { running, error, .. } => (*running, *error, None),
                _ => return Err(format!("{name}: неожиданный runtime")),
            }
        };
        wait_done(name, running, error, progress)
    };
    run_step("text-encoder", n_enc, ltx::text_encoder::start)?;
    run_step("lipdub", n_lip, ltx::lipdub::start)?;
    run_step("vae-decode", n_vae, ltx::vae_decode::start)?;
    run_step("audio-decode", n_aud, ltx::audio_decode::start)?;
    run_step("video-save", n_save, ltx::video_save::start)?;

    let meta = std::fs::metadata(out).map_err(|e| format!("выходной файл: {e}"))?;
    if meta.len() == 0 {
        return Err("выходной mp4 пуст".into());
    }
    eprintln!("[ltx_smoke] lipdub OK: mp4 {} байт", meta.len());
    Ok(())
}

/// a2v-граф: Checkpoint(+upscaler) + TextEncoder + AudioInput → A2V → VAE/Audio
/// Decode → Save.
#[allow(clippy::too_many_arguments)]
fn run_a2v(
    ckpt: &PathBuf,
    gemma: &PathBuf,
    upscaler: &PathBuf,
    audio: &str,
    out: &str,
    width: u32,
    height: u32,
    dur: f32,
) -> Result<(), String> {
    let ctx = NodeEditorCtx::new();
    let zero = syngui::core::Point::new(0.0, 0.0);
    let n_ckpt = ctx.add_node(NodeKind::LtxCheckpoint, zero);
    let n_enc = ctx.add_node(NodeKind::LtxTextEncoder, zero);
    let n_ain = ctx.add_node(NodeKind::LtxAudioInput, zero);
    let n_a2v = ctx.add_node(NodeKind::LtxA2V, zero);
    let n_vae = ctx.add_node(NodeKind::LtxVaeDecode, zero);
    let n_aud = ctx.add_node(NodeKind::LtxAudioDecode, zero);
    let n_save = ctx.add_node(NodeKind::LtxVideoSave, zero);

    {
        let inst = node(&ctx, n_ckpt);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxCheckpoint { model_path, gemma_dir, upscaler_path, .. } = &*g {
            model_path.set(Some(ckpt.clone()));
            gemma_dir.set(Some(gemma.clone()));
            upscaler_path.set(Some(upscaler.clone()));
        }
    }
    {
        let inst = node(&ctx, n_enc);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxTextEncoder { prompt_field, .. } = &*g {
            prompt_field.set("a dynamic music video, vibrant motion".into());
        }
    }
    {
        let inst = node(&ctx, n_ain);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxAudioInput { audio_path } = &*g {
            audio_path.set(Some(PathBuf::from(audio)));
        }
    }
    {
        let inst = node(&ctx, n_a2v);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxA2V { width: w, height: h, duration_seconds, .. } = &*g {
            w.set(width);
            h.set(height);
            duration_seconds.set(dur);
        }
    }
    {
        let inst = node(&ctx, n_save);
        let g = inst.runtime.lock().map_err(|_| "lock")?;
        if let NodeRuntime::LtxVideoSave { path, .. } = &*g {
            path.set(out.to_string());
        }
    }

    let conn = |from: NodeId, fp: &'static str, to: NodeId, tp: &'static str| Connection {
        from_node: from, from_port: fp, to_node: to, to_port: tp,
    };
    ctx.connections.set(vec![
        conn(n_ckpt, "model", n_enc, "model"),
        conn(n_ckpt, "model", n_a2v, "model"),
        conn(n_ckpt, "model", n_vae, "model"),
        conn(n_ckpt, "model", n_aud, "model"),
        conn(n_enc, "video_encoding", n_a2v, "video_encoding"),
        conn(n_enc, "audio_encoding", n_a2v, "audio_encoding"),
        conn(n_ain, "audio", n_a2v, "audio"),
        conn(n_a2v, "video_latent", n_vae, "video_latent"),
        conn(n_a2v, "audio_tokens", n_aud, "audio_tokens"),
        conn(n_vae, "frames", n_save, "frames"),
        conn(n_aud, "audio", n_save, "audio"),
    ]);

    eprintln!("[ltx_smoke] a2v {}×{} {:.1}s → {out}", width, height, dur);
    let run_step = |name: &str, id: NodeId, start: fn(&NodeInstance, &NodeEditorCtx)| -> Result<(), String> {
        refresh(&ctx);
        let inst = node(&ctx, id);
        start(&inst, &ctx);
        let (running, error, progress) = {
            let g = inst.runtime.lock().map_err(|_| "lock")?;
            match &*g {
                NodeRuntime::LtxTextEncoder { running, error, progress_pct, .. }
                | NodeRuntime::LtxA2V { running, error, progress_pct, .. }
                | NodeRuntime::LtxVaeDecode { running, error, progress_pct, .. }
                | NodeRuntime::LtxVideoSave { running, error, progress_pct, .. } => {
                    (*running, *error, Some(*progress_pct))
                }
                NodeRuntime::LtxAudioDecode { running, error, .. } => (*running, *error, None),
                _ => return Err(format!("{name}: неожиданный runtime")),
            }
        };
        wait_done(name, running, error, progress)
    };
    run_step("text-encoder", n_enc, ltx::text_encoder::start)?;
    run_step("a2v", n_a2v, ltx::a2v::start)?;
    run_step("vae-decode", n_vae, ltx::vae_decode::start)?;
    run_step("audio-decode", n_aud, ltx::audio_decode::start)?;
    run_step("video-save", n_save, ltx::video_save::start)?;

    let meta = std::fs::metadata(out).map_err(|e| format!("выходной файл: {e}"))?;
    if meta.len() == 0 {
        return Err("выходной mp4 пуст".into());
    }
    eprintln!("[ltx_smoke] a2v OK: mp4 {} байт", meta.len());
    Ok(())
}

fn main() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .try_init();
    syngui::signal::init_main_thread();
    match run() {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("[ltx_smoke] ОШИБКА: {e}");
            std::process::exit(1);
        }
    }
}
