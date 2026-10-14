//! Перевод графов с прежними родными нодами моделей (FLUX.1, FLUX.2,
//! Qwen-Image, Qwen-Image 2.1, SDXL, MiniMax-H3) в стадии ядра
//! synaptix-flow, которые исполняет служба synaptixd. Работает по JSON до
//! разбора нод: рабочая область, свои шаблоны и встроенные шаблоны
//! (`builtin/*.json` получены этим же переводом).
//!
//! Нода к ноде: чекпойнт → «Модель», кодировщик → «Кодировщик описания»,
//! пустой латент, картинка в латент, референсы, сэмплер, декодер. У H3
//! кодировщик получает размер ролика проводом от «Латента ролика» (раньше
//! его брали сэмплер и референсы), ключевой кадр становится файлом (или
//! провод идёт прямо от картинки), референсы — списком файлов, сохранение
//! ролика — наложением звука. Музыка: ACE-Step (чекпойнт, запись в латент,
//! генерация) и YuE2 (чекпойнт, песня, декодер, транскрипция) — те же
//! ноды стадиями. Речь (OmniVoice, VoxCPM2, VibeVoice, GigaAM, Sortformer) и
//! LLM — ноды службы; Syn Checkpoint уходит, его файл — поле «Модель».

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{json, Value};

/// Модель по умолчанию для чекпойнта без выбранного файла (id каталога службы).
fn default_model(kind: &str, state: &Value) -> &'static str {
    match kind {
        "flux_checkpoint" => "flux-1-dev",
        "flux2_checkpoint" => "flux-2-klein-4b",
        "qwen_image_checkpoint" => "qwen-image-edit-2511",
        "qwen_image21_checkpoint" => "qwen-image-2-1",
        "sdxl_checkpoint" => "sdxl-base-1-0",
        "ltx_checkpoint" => "ltx-2-3-22b-distilled-1-1",
        _ if state.get("variant_idx").and_then(Value::as_u64) == Some(1) => "minimax-h3-ref2va",
        _ => "minimax-h3-fl2va",
    }
}

fn text(s: impl Into<String>) -> Value {
    json!({"type": "Text", "value": s.into()})
}

fn float(v: f64) -> Value {
    json!({"type": "Float", "value": v})
}

fn boolean(b: bool) -> Value {
    json!({"type": "Bool", "value": b})
}

fn choice(i: usize) -> Value {
    json!({"type": "Choice", "value": i})
}

/// Индекс варианта поля ядра (`synaptix_flow::ops::stages::QUANTS` и т. п.).
fn option(options: &[&str], v: &str) -> usize {
    options.iter().position(|o| *o == v).unwrap_or(0)
}

/// Квант по индексу списка родной ноды → вариант ядра. NVFP4 медиамоделям
/// не ставится (политика: медиа — только MXFP8) — пусто, формат файла.
fn quant(kind: &str, idx: u64) -> &'static str {
    let names: &[&str] = match kind {
        "sdxl_checkpoint" => &["", "mxfp8", ""],
        _ => &["", "mxfp8", "bf16"],
    };
    names.get(idx as usize).copied().unwrap_or("")
}

/// Квант музыкальной модели по списку родной ноды (none, nvfp4, mxfp8):
/// NVFP4 медиамоделям не ставится — пусто, формат файла.
fn music_quant(idx: u64) -> &'static str {
    if idx == 2 {
        "mxfp8"
    } else {
        ""
    }
}

/// Новый вид, поля и переименование портов (вход, выход) родной ноды.
struct Plan {
    kind: &'static str,
    fields: BTreeMap<String, Value>,
    inputs: &'static [(&'static str, &'static str)],
    outputs: &'static [(&'static str, &'static str)],
}

fn num(state: &Value, k: &str) -> Option<f64> {
    state.get(k).and_then(Value::as_f64)
}

fn plan(kind: &str, state: &Value) -> Option<Plan> {
    use synaptix_flow::ops::stages::{FITS, QUANTS};
    let mut f = BTreeMap::new();
    let p = |kind, fields, inputs, outputs| Some(Plan { kind, fields, inputs, outputs });
    match kind {
        "flux_checkpoint" | "flux2_checkpoint" | "qwen_image_checkpoint" | "qwen_image21_checkpoint" | "sdxl_checkpoint" | "h3_checkpoint" => {
            let model = state.get("model_path").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| default_model(kind, state).into());
            f.insert("model".into(), text(model));
            let q = if kind == "h3_checkpoint" { num(state, "quant_dit_idx") } else { num(state, "quant_idx") };
            if let Some(q) = q {
                f.insert("quant".into(), choice(option(QUANTS, quant(kind, q as u64))));
            }
            if let (true, Some(i)) = (kind == "qwen_image21_checkpoint", num(state, "resolution_idx")) {
                let r = ["512", "768", "1024", "1536", "2048"].get(i as usize).copied().unwrap_or("");
                f.insert("resolution".into(), choice(option(synaptix_flow::ops::stages::RESOLUTIONS, r)));
            }
            if kind == "h3_checkpoint" {
                if let Some(v) = state.get("lora_path").and_then(Value::as_str).filter(|v| !v.is_empty()) {
                    f.insert("lora".into(), text(v));
                }
                if let Some(v) = num(state, "lora_strength") {
                    f.insert("lora_strength".into(), float(v));
                }
            }
            let flow = if kind == "h3_checkpoint" { "flow_h3_model" } else { "flow_image_model" };
            p(flow, f, &[], &[("model", "model")])
        }
        "flux_text_encoder" | "flux2_text_encoder" | "qwen_image_text_encoder" | "qwen_image21_text_encoder" | "sdxl_text_encoder" => {
            if let Some(i) = num(state, "seq_len_idx") {
                f.insert("seq_len".into(), float([0.0, 256.0, 512.0].get(i as usize).copied().unwrap_or(0.0)));
            }
            p("flow_image_encode", f, &[("model", "model"), ("prompt", "prompt"), ("negative", "negative"), ("references", "refs")], &[("conditioning", "cond")])
        }
        "flux_empty_latent" => {
            f.insert("width".into(), float(num(state, "width").unwrap_or(1024.0)));
            f.insert("height".into(), float(num(state, "height").unwrap_or(1024.0)));
            p("flow_image_latent", f, &[], &[("latent", "latent")])
        }
        "flux_vae_encode" | "flux2_vae_encode" | "sdxl_vae_encode" => {
            let fit = if num(state, "resize_idx") == Some(0.0) { "stretch" } else { "cover" };
            f.insert("fit".into(), choice(option(FITS, fit)));
            p("flow_image_to_latent", f, &[("model", "model"), ("image", "image"), ("size", "size")], &[("latent", "latent")])
        }
        "flux2_reference" | "qwen_image_reference" | "qwen_image21_reference" => {
            p("flow_image_refs", f, &[("model", "model"), ("image", "images"), ("references", "refs")], &[("references", "refs")])
        }
        "flux_sampler" | "flux2_sampler" | "qwen_image_sampler" | "qwen_image21_sampler" | "sdxl_sampler" => {
            for (from, to) in [("steps", "steps"), ("guidance", "guidance"), ("cfg", "guidance"), ("seed", "seed"), ("denoise", "denoise")] {
                if let Some(v) = num(state, from) {
                    f.insert(to.into(), float(v));
                }
            }
            p("flow_image_sample", f, &[("model", "model"), ("conditioning", "cond"), ("latent", "latent"), ("references", "refs")], &[("latent", "latent")])
        }
        "flux_vae_decode" | "flux2_vae_decode" | "qwen_image_vae_decode" | "qwen_image21_vae_decode" | "sdxl_vae_decode" => {
            p("flow_image_decode", f, &[("model", "model"), ("latent", "latent")], &[("image", "image")])
        }
        "h3_text_encoder" => p(
            "flow_h3_encode",
            f,
            &[("model", "model"), ("prompt", "prompt"), ("keyframe", "first_frame"), ("keyframe_last", "last_frame"), ("refs", "refs")],
            &[("conditioning", "cond")],
        ),
        "h3_empty_latent_av" => {
            f.insert("width".into(), float(num(state, "width").unwrap_or(1344.0)));
            f.insert("height".into(), float(num(state, "height").unwrap_or(768.0)));
            f.insert("seconds".into(), float(num(state, "duration_seconds").unwrap_or(5.0)));
            p("flow_h3_latent", f, &[], &[("av_latent", "latent")])
        }
        "h3_keyframe" => {
            let path = state.get("image_path").and_then(Value::as_str).unwrap_or("");
            f.insert("path".into(), text(path));
            p("flow_file", f, &[], &[("keyframe", "file")])
        }
        "h3_references" => {
            let items: Vec<&str> = state.get("items").and_then(Value::as_array).map(|a| a.iter().filter_map(|i| i.get("path").and_then(Value::as_str)).collect()).unwrap_or_default();
            f.insert("items".into(), text(items.join("\n")));
            p("flow_list", f, &[], &[("refs", "list")])
        }
        "h3_sampler" => {
            for (from, to) in [("steps", "steps"), ("cfg_scale", "guidance"), ("seed", "seed")] {
                if let Some(v) = num(state, from) {
                    f.insert(to.into(), float(v));
                }
            }
            p("flow_h3_sample", f, &[("model", "model"), ("conditioning", "cond")], &[("video_latent", "latent"), ("audio_latent", "latent")])
        }
        "h3_vae_decode" => p("flow_h3_decode_video", f, &[("model", "model"), ("video_latent", "latent")], &[("frames", "video")]),
        "h3_audio_decode" => p("flow_h3_decode_audio", f, &[("model", "model"), ("audio_latent", "latent")], &[("audio", "audio")]),
        "h3_video_save" => {
            f.insert("mode".into(), choice(0));
            p("flow_mux", f, &[("frames", "video"), ("audio", "audio")], &[("path", "video")])
        }
        "ltx_checkpoint" => {
            let model = state.get("model_path").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| default_model(kind, state).into());
            f.insert("model".into(), text(model));
            if let Some(q) = num(state, "quant_dit_idx") {
                f.insert("quant".into(), choice(option(QUANTS, quant(kind, q as u64))));
            }
            for (from, to) in [("lora_path", "lora"), ("depth_model_path", "depth")] {
                if let Some(v) = state.get(from).and_then(Value::as_str).filter(|v| !v.is_empty()) {
                    f.insert(to.into(), text(v));
                }
            }
            if let Some(v) = num(state, "lora_strength") {
                f.insert("lora_strength".into(), float(v));
            }
            p("flow_ltx_model", f, &[], &[("model", "model")])
        }
        "ltx_text_encoder" => {
            if let Some(t) = state.get("prompt").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                f.insert("prompt".into(), text(t));
            }
            p("flow_ltx_encode", f, &[("model", "model"), ("prompt", "prompt")], &[("video_encoding", "cond"), ("audio_encoding", "cond")])
        }
        "ltx_nag_prompt" => {
            if let Some(t) = state.get("prompt").and_then(Value::as_str) {
                f.insert("prompt".into(), text(t));
            }
            for k in ["scale", "alpha", "tau"] {
                if let Some(v) = num(state, k) {
                    f.insert(k.into(), float(v));
                }
            }
            p("flow_ltx_nag", f, &[("model", "model"), ("text", "prompt")], &[("nag", "nag")])
        }
        "ltx_image" => {
            if let Some(v) = num(state, "strength") {
                f.insert("strength".into(), float(v));
            }
            if let Some(v) = num(state, "frame_idx") {
                f.insert("frame".into(), float(v));
            }
            p("flow_ltx_image", f, &[("image", "image")], &[("image_cond", "image")])
        }
        "ltx_video_input" | "ltx_audio_input" => {
            let key = if kind == "ltx_video_input" { "video_path" } else { "audio_path" };
            f.insert("path".into(), text(state.get(key).and_then(Value::as_str).unwrap_or("")));
            p("flow_file", f, &[], &[("video", "file"), ("audio", "file")])
        }
        "ltx_sampler_stage1" | "ltx_retake" | "ltx_ic_lora" | "ltx_lipdub" | "ltx_a2v" | "ltx_a2_v" => {
            for (from, to) in [("width", "width"), ("height", "height"), ("duration_seconds", "seconds"), ("seed", "seed"), ("retake_start", "start"), ("retake_end", "end"), ("downscale", "downscale"), ("ref_strength", "strength"), ("canny_low", "canny_low"), ("canny_high", "canny_high")] {
                if let Some(v) = num(state, from) {
                    f.insert(to.into(), float(v));
                }
            }
            for (from, to) in [("fps_idx", "fps"), ("control_idx", "control")] {
                if let Some(v) = num(state, from) {
                    f.insert(to.into(), choice(v as usize));
                }
            }
            if kind == "ltx_lipdub" {
                f.remove("seed");
            }
            let flow = match kind {
                "ltx_sampler_stage1" => "flow_ltx_stage1",
                "ltx_retake" => "flow_ltx_retake",
                "ltx_ic_lora" => "flow_ltx_ic_lora",
                "ltx_lipdub" => "flow_ltx_lipdub",
                _ => "flow_ltx_a2v",
            };
            p(
                flow,
                f,
                &[("model", "model"), ("video_encoding", "cond"), ("audio_encoding", "cond"), ("nag", "nag"), ("image_cond", "image"), ("video", "video"), ("ref_video", "video"), ("audio", "audio")],
                &[("video_latent", "latent"), ("audio_tokens", "latent")],
            )
        }
        "ltx_upscale" => p("flow_ltx_upscale", f, &[("model", "model"), ("video_latent", "latent")], &[("video_latent", "latent")]),
        "ltx_sampler_stage2" => {
            if let Some(v) = num(state, "seed") {
                f.insert("seed".into(), float(v));
            }
            // Звуковые токены Stage1 едут в латенте апскейла.
            p("flow_ltx_stage2", f, &[("model", "model"), ("video_latent", "latent"), ("video_encoding", "cond"), ("audio_encoding", "cond"), ("image_cond", "image")], &[("video_latent", "latent"), ("audio_tokens", "latent")])
        }
        "ltx_vae_decode" => p("flow_ltx_decode_video", f, &[("model", "model"), ("video_latent", "latent")], &[("frames", "video")]),
        "ltx_audio_decode" => p("flow_ltx_decode_audio", f, &[("model", "model"), ("audio_tokens", "latent")], &[("audio", "audio")]),
        "ltx_video_save" => {
            f.insert("mode".into(), choice(0));
            p("flow_mux", f, &[("frames", "video"), ("audio", "audio")], &[("path", "video")])
        }
        "ace_step_checkpoint" => {
            // DiT: файл (путь — как есть; голое имя — модель библиотеки по варианту).
            let dit = state.get("dit_path").and_then(Value::as_str).unwrap_or("");
            let model = if dit.starts_with('/') {
                dit.to_string()
            } else if dit.contains("turbo") {
                "acestep-v15-xl-turbo".into()
            } else {
                "acestep-v15-xl-base".into()
            };
            f.insert("model".into(), text(model));
            if state.get("lm_path").and_then(Value::as_str).unwrap_or("").contains("1.7b") {
                f.insert("lm".into(), choice(1));
            }
            if let Some(q) = num(state, "quant_dit_idx") {
                f.insert("quant".into(), choice(option(QUANTS, music_quant(q as u64))));
            }
            p("flow_acestep_model", f, &[], &[("model", "model")])
        }
        "ace_step_vae_encode" => {
            for (from, to) in [("chunk_seconds", "chunk"), ("overlap_seconds", "overlap")] {
                if let Some(v) = num(state, from) {
                    f.insert(to.into(), float(v));
                }
            }
            p("flow_acestep_encode", f, &[("model", "model"), ("audio", "audio")], &[("latent", "latent")])
        }
        "ace_step_generate" => {
            use synaptix_flow::ops::stages::{DCW_MODES, NORMS};
            let auto = state.get("preset").and_then(Value::as_str).unwrap_or("Auto") == "Auto";
            let mut pairs = vec![
                ("duration_seconds", "seconds"),
                ("seed", "seed"),
                ("temperature", "temperature"),
                ("top_p", "top_p"),
                ("top_k", "top_k"),
                ("min_p", "min_p"),
                ("lm_cfg_scale", "lm_guidance"),
                ("bpm", "bpm"),
                ("dcw_scaler", "dcw_scaler"),
                ("dcw_high_scaler", "dcw_high_scaler"),
                ("retake_variance", "retake_variance"),
                ("retake_seed", "retake_seed"),
                ("repaint_start_sec", "repaint_start"),
                ("repaint_end_sec", "repaint_end"),
                ("repaint_strength", "repaint_strength"),
                ("edit_n_min", "edit_n_min"),
                ("edit_n_max", "edit_n_max"),
            ];
            // Пресет Auto — умолчания варианта DiT (нули), иначе заданные значения.
            if !auto {
                pairs.extend([("infer_steps", "steps"), ("cfg_scale", "guidance"), ("flow_match_shift", "shift")]);
            }
            for (from, to) in pairs {
                if let Some(v) = num(state, from) {
                    f.insert(to.into(), float(v));
                }
            }
            for (from, to) in [("mode_idx", "mode"), ("keyscale_idx", "keyscale"), ("timesig_idx", "timesig"), ("track_idx", "track")] {
                if let Some(v) = num(state, from) {
                    f.insert(to.into(), choice(v as usize));
                }
            }
            for (from, to) in [("use_cot", "cot"), ("use_ar", "ar")] {
                if let Some(v) = state.get(from).and_then(Value::as_bool) {
                    f.insert(to.into(), boolean(v));
                }
            }
            if let Some(n) = state.get("norm_mode").and_then(Value::as_str) {
                f.insert("norm".into(), choice(option(NORMS, if n == "none" { "off" } else { n })));
            }
            if state.get("enable_dcw").and_then(Value::as_bool) == Some(true) {
                let m = state.get("dcw_mode").and_then(Value::as_str).unwrap_or("Double").to_ascii_lowercase();
                f.insert("dcw".into(), choice(option(DCW_MODES, &m)));
            }
            p("flow_acestep_generate", f, &[("model", "model"), ("tags", "prompt"), ("lyrics", "lyrics"), ("src_latent", "latent")], &[("audio", "audio"), ("latent", "latent")])
        }
        "yue2_checkpoint" => {
            let model = state.get("model_path").and_then(Value::as_str).filter(|m| m.starts_with('/')).unwrap_or("yue2-3b");
            f.insert("model".into(), text(model));
            if let Some(q) = num(state, "quant_idx") {
                f.insert("quant".into(), choice(option(QUANTS, music_quant(q as u64))));
            }
            p("flow_yue2_model", f, &[], &[("model", "model")])
        }
        "yue2_generate" => {
            for (from, to) in [
                ("seconds", "seconds"),
                ("ode_steps", "steps"),
                ("cfg_scale", "guidance"),
                ("seed", "seed"),
                ("temperature", "temperature"),
                ("top_p", "top_p"),
                ("top_k", "top_k"),
                ("repetition_penalty", "repetition_penalty"),
                ("vae_core_frames", "core_frames"),
            ] {
                if let Some(v) = num(state, from) {
                    f.insert(to.into(), float(v));
                }
            }
            if let Some(v) = num(state, "cot_idx") {
                f.insert("cot".into(), choice(v as usize));
            }
            p("flow_yue2_generate", f, &[("model", "model"), ("style", "prompt"), ("lyrics", "lyrics"), ("abc", "abc")], &[("audio", "audio"), ("score", "score"), ("latent", "latent")])
        }
        "yue2_vae_decode" => {
            if let Some(v) = num(state, "vae_core_frames") {
                f.insert("core_frames".into(), float(v));
            }
            p("flow_yue2_decode", f, &[("model", "model"), ("latent", "latent")], &[("audio", "audio")])
        }
        "yue2_transcribe" => {
            for (from, to) in [("mode_idx", "mode"), ("voices_idx", "voices")] {
                if let Some(v) = num(state, from) {
                    f.insert(to.into(), choice(v as usize));
                }
            }
            p("flow_yue2_transcribe", f, &[("model", "model"), ("audio", "audio")], &[("score", "score")])
        }
        "omni_voice" | "vox_cpm2" | "vibe_voice" | "asr_gigaam" | "sortformer_diarizer" | "llm" => {
            // Модель: файл Syn Checkpoint на входе (`__model`, см. [`migrate`]),
            // своя прежняя, иначе модель библиотеки по умолчанию.
            let model = ["__model", "model_path"]
                .iter()
                .find_map(|k| state.get(*k).and_then(Value::as_str).filter(|m| !m.is_empty()))
                .unwrap_or(match kind {
                    "omni_voice" => "omnivoice",
                    "vox_cpm2" => "voxcpm2",
                    "vibe_voice" => "vibevoice-1-5b",
                    "asr_gigaam" => "gigaam-v3",
                    _ => "",
                });
            if !model.is_empty() {
                f.insert("model".into(), text(model));
            }
            let (texts, nums, bools): (&[(&str, &str)], &[(&str, &str)], &[(&str, &str)]) = match kind {
                "omni_voice" => (
                    &[("instruct", "instruct"), ("ref_text", "ref_text"), ("language", "language")],
                    &[("num_step", "steps"), ("guidance_scale", "guidance"), ("t_shift", "t_shift"), ("speed", "speed"), ("seed", "seed")],
                    &[],
                ),
                "vox_cpm2" => (&[("prompt_text", "prompt_text")], &[("cfg_value", "guidance"), ("n_timesteps", "steps"), ("max_len", "max_len"), ("seed", "seed")], &[]),
                "vibe_voice" => (&[("script", "script")], &[("cfg_value", "guidance"), ("ddpm_steps", "steps"), ("max_length_times", "max_length_times"), ("seed", "seed")], &[]),
                "sortformer_diarizer" => (&[], &[("threshold", "threshold")], &[("allow_overlap", "overlap")]),
                "llm" => (
                    &[("system_prompt", "system")],
                    &[("context", "context"), ("max_tokens", "max_tokens"), ("temperature", "temperature"), ("top_k", "top_k"), ("top_p", "top_p"), ("min_p", "min_p"), ("repetition_penalty", "repeat_penalty")],
                    &[("think", "think")],
                ),
                _ => (&[], &[], &[]),
            };
            for (from, to) in texts {
                if let Some(v) = state.get(*from).and_then(Value::as_str).filter(|v| !v.is_empty()) {
                    f.insert((*to).into(), text(v));
                }
            }
            for (from, to) in nums {
                if let Some(v) = num(state, from) {
                    f.insert((*to).into(), float(v));
                }
            }
            for (from, to) in bools {
                if let Some(v) = state.get(*from).and_then(Value::as_bool) {
                    f.insert((*to).into(), boolean(v));
                }
            }
            match kind {
                "omni_voice" => p("flow_omnivoice", f, &[("text", "text"), ("ref_audio", "ref_audio"), ("ref_text", "ref_text")], &[("audio", "audio")]),
                "vox_cpm2" => p("flow_voxcpm", f, &[("text", "text"), ("ref_audio", "ref_audio"), ("prompt_audio", "prompt_audio"), ("prompt_text", "prompt_text")], &[("audio", "audio")]),
                "vibe_voice" => p(
                    "flow_vibevoice",
                    f,
                    &[("script", "script"), ("voice1", "voice1"), ("voice2", "voice2"), ("voice3", "voice3"), ("voice4", "voice4")],
                    &[("audio", "audio")],
                ),
                "asr_gigaam" => p("flow_transcribe", f, &[("in", "file")], &[("out", "text")]),
                "sortformer_diarizer" => p("flow_diarize", f, &[("in", "audio")], &[("out", "json")]),
                _ => {
                    // Сид 0 прежней ноды — случайный; у ядра это -1.
                    if num(state, "seed").is_some_and(|v| v > 0.0) {
                        f.insert("seed".into(), float(num(state, "seed").unwrap_or(0.0)));
                    }
                    p("flow_llm", f, &[("prompt", "prompt"), ("system", "system")], &[("answer", "text")])
                }
            }
        }
        _ => None,
    }
}

fn id_of(v: &Value, k: &str) -> Option<u64> {
    v.get(k).and_then(Value::as_u64)
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn conn(from: u64, fp: &str, to: u64, tp: &str) -> Value {
    json!({"from_node": from, "from_port": fp, "to_node": to, "to_port": tp})
}

/// Перевести родные ноды графа. `true` — что-то изменилось.
pub fn migrate(nodes: &mut Vec<Value>, conns: &mut Vec<Value>) -> bool {
    let kinds: HashMap<u64, String> = nodes.iter().filter_map(|n| Some((id_of(n, "id")?, str_of(n, "kind").to_string()))).collect();
    if !kinds.values().any(|k| k == "syn_checkpoint" || plan(k, &Value::Null).is_some()) {
        return false;
    }
    // Syn Checkpoint больше нет: его файл становится полем «Модель» нод на его
    // выходе, сам он уходит из графа.
    let checkpoint: HashMap<u64, String> = nodes
        .iter()
        .filter(|n| str_of(n, "kind") == "syn_checkpoint")
        .filter_map(|n| Some((id_of(n, "id")?, n["state"]["data"].get("model_path").and_then(Value::as_str).unwrap_or("").to_string())))
        .collect();
    if !checkpoint.is_empty() {
        for c in conns.iter() {
            let (Some(from), Some(to)) = (id_of(c, "from_node"), id_of(c, "to_node")) else { continue };
            let Some(path) = checkpoint.get(&from).filter(|p| !p.is_empty()) else { continue };
            if let Some(n) = nodes.iter_mut().find(|n| id_of(n, "id") == Some(to)) {
                if !n["state"].is_object() {
                    n["state"] = json!({"data": {}});
                }
                if !n["state"]["data"].is_object() {
                    n["state"]["data"] = json!({});
                }
                n["state"]["data"]["__model"] = json!(path);
            }
        }
        nodes.retain(|n| str_of(n, "kind") != "syn_checkpoint");
        conns.retain(|c| id_of(c, "from_node").is_none_or(|f| !checkpoint.contains_key(&f)) && id_of(c, "to_node").is_none_or(|t| !checkpoint.contains_key(&t)));
    }
    // Ключевой кадр H3 с проводом картинки — сам провод идёт дальше.
    let mut bypass: HashMap<u64, (u64, String)> = HashMap::new();
    for c in conns.iter() {
        let (Some(to), Some(from)) = (id_of(c, "to_node"), id_of(c, "from_node")) else { continue };
        if kinds.get(&to).map(String::as_str) == Some("h3_keyframe") && str_of(c, "to_port") == "image" {
            bypass.insert(to, (from, str_of(c, "from_port").to_string()));
        }
    }
    // Кодировщику H3 — размер ролика: латент, что шёл в сэмплер этого кодировщика.
    let mut sampler_latent: HashMap<u64, u64> = HashMap::new();
    let mut encoder_sampler: HashMap<u64, u64> = HashMap::new();
    for c in conns.iter() {
        let (Some(from), Some(to)) = (id_of(c, "from_node"), id_of(c, "to_node")) else { continue };
        match (kinds.get(&from).map(String::as_str), kinds.get(&to).map(String::as_str), str_of(c, "to_port")) {
            (Some("h3_empty_latent_av"), Some("h3_sampler"), "av_latent") => {
                sampler_latent.insert(to, from);
            }
            (Some("h3_text_encoder"), Some("h3_sampler"), "conditioning") => {
                encoder_sampler.insert(from, to);
            }
            _ => {}
        }
    }
    // Крупные референсы H3 («max») — теперь поле кодировщика.
    let refs_max = nodes.iter().any(|n| str_of(n, "kind") == "h3_references" && n["state"]["data"].get("image_size_idx").and_then(Value::as_u64) == Some(1));
    // H3 с референсами — вариант Ref2VA.
    if kinds.values().any(|k| k == "h3_references") {
        for n in nodes.iter_mut().filter(|n| str_of(n, "kind") == "h3_checkpoint") {
            if n["state"]["data"].get("model_path").and_then(Value::as_str).unwrap_or("").is_empty() {
                n["state"] = json!({"kind": "H3Checkpoint", "data": {"variant_idx": 1}});
            }
        }
    }
    // Кадр-условие LTX с файлом — нода «Файл» перед ним.
    let mut next_id = kinds.keys().max().copied().unwrap_or(0) + 1;
    let mut new_nodes = Vec::new();
    for n in nodes.iter() {
        let (Some(id), "ltx_image") = (id_of(n, "id"), str_of(n, "kind")) else { continue };
        let wired = conns.iter().any(|c| id_of(c, "to_node") == Some(id) && str_of(c, "to_port") == "image");
        let path = n["state"]["data"].get("image_path").and_then(Value::as_str).unwrap_or("");
        if wired {
            continue;
        }
        let (x, y) = (n["pos"]["x"].as_f64().unwrap_or(0.0), n["pos"]["y"].as_f64().unwrap_or(0.0));
        new_nodes.push(json!({"id": next_id, "kind": "flow_file", "pos": {"x": x - 380.0, "y": y}, "fields": {"path": text(path)}, "state": null}));
        conns.push(conn(next_id, "file", id, "image"));
        next_id += 1;
    }
    nodes.extend(new_nodes);
    // Проигрыватель кадров H3/LTX → готовый ролик: сохранение (наложение звука)
    // тех же декодеров или ролик декодера.
    let mut save_of: HashMap<u64, u64> = HashMap::new();
    for c in conns.iter() {
        let (Some(from), Some(to)) = (id_of(c, "from_node"), id_of(c, "to_node")) else { continue };
        if matches!(kinds.get(&to).map(String::as_str), Some("h3_video_save" | "ltx_video_save")) && matches!(kinds.get(&from).map(String::as_str), Some("h3_vae_decode" | "ltx_vae_decode")) {
            save_of.insert(from, to);
        }
    }
    let mut player = Vec::new();
    conns.retain(|c| {
        let (Some(from), Some(to)) = (id_of(c, "from_node"), id_of(c, "to_node")) else { return true };
        if kinds.get(&to).map(String::as_str) != Some("ffmpeg_player") {
            return true;
        }
        match (kinds.get(&from).map(String::as_str), str_of(c, "to_port")) {
            (Some("h3_vae_decode" | "ltx_vae_decode"), "frames") => {
                player.push(match save_of.get(&from) {
                    Some(&save) => conn(save, "path", to, "source"),
                    None => conn(from, "frames", to, "source"),
                });
                false
            }
            (Some("h3_audio_decode" | "ltx_audio_decode"), "audio") => false,
            _ => true,
        }
    });
    conns.extend(player);
    let latents: Vec<u64> = kinds.iter().filter(|(_, k)| *k == "h3_empty_latent_av").map(|(i, _)| *i).collect();
    let mut extra = Vec::new();
    for (id, k) in &kinds {
        if k != "h3_text_encoder" {
            continue;
        }
        let lat = encoder_sampler.get(id).and_then(|s| sampler_latent.get(s)).copied().or(if latents.len() == 1 { Some(latents[0]) } else { None });
        if let Some(l) = lat {
            extra.push(conn(l, "av_latent", *id, "latent_size"));
        }
    }
    conns.extend(extra);

    let mut plans: HashMap<u64, Plan> = HashMap::new();
    for n in nodes.iter_mut() {
        let Some(id) = id_of(n, "id") else { continue };
        let state = n.get("state").and_then(|s| s.get("data")).cloned().unwrap_or(Value::Null);
        let Some(pl) = plan(str_of(n, "kind"), &state) else { continue };
        n["kind"] = json!(pl.kind);
        n["fields"] = json!(pl.fields);
        n["state"] = Value::Null;
        plans.insert(id, pl);
    }
    nodes.retain(|n| id_of(n, "id").map(|i| !bypass.contains_key(&i)).unwrap_or(true));
    if refs_max {
        for n in nodes.iter_mut().filter(|n| str_of(n, "kind") == "flow_h3_encode") {
            n["fields"]["ref_size"] = choice(1);
        }
    }
    // LipDub и IC-LoRA работают с адаптером модели: без своего — компонент службы.
    let adapter = if nodes.iter().any(|n| str_of(n, "kind") == "flow_ltx_lipdub") {
        Some("lipdub")
    } else if nodes.iter().any(|n| str_of(n, "kind") == "flow_ltx_ic_lora") {
        Some("union")
    } else {
        None
    };
    if let Some(a) = adapter {
        for n in nodes.iter_mut().filter(|n| str_of(n, "kind") == "flow_ltx_model") {
            if n["fields"].get("lora").is_none() {
                n["fields"]["lora"] = text(a);
            }
        }
    }

    let mut out = Vec::with_capacity(conns.len());
    let mut taken: HashSet<(u64, String)> = HashSet::new();
    for c in conns.iter() {
        let (Some(mut from), Some(to)) = (id_of(c, "from_node"), id_of(c, "to_node")) else { continue };
        let mut fp = str_of(c, "from_port").to_string();
        let mut tp = str_of(c, "to_port").to_string();
        if bypass.contains_key(&to) {
            continue;
        }
        if let Some((src, port)) = bypass.get(&from) {
            from = *src;
            fp = port.clone();
        } else if let Some(pl) = plans.get(&from) {
            match pl.outputs.iter().find(|(o, _)| *o == fp) {
                Some((_, n)) => fp = (*n).to_string(),
                None => continue,
            }
        }
        if tp == "latent_size" {
            tp = "latent".into();
        } else if let Some(pl) = plans.get(&to) {
            match pl.inputs.iter().find(|(i, _)| *i == tp) {
                Some((_, n)) => tp = (*n).to_string(),
                None => continue,
            }
        }
        // На вход — один провод.
        if !taken.insert((to, tp.clone())) {
            continue;
        }
        out.push(conn(from, &fp, to, &tp));
    }
    *conns = out;
    true
}

/// Перевести граф в JSON-объекте с `nodes`/`connections` (шаблон, вкладка).
pub fn migrate_graph(graph: &mut Value) -> bool {
    let (Some(nodes), Some(conns)) = (graph.get("nodes").cloned(), graph.get("connections").cloned()) else {
        return false;
    };
    let (Value::Array(mut nodes), Value::Array(mut conns)) = (nodes, conns) else { return false };
    if !migrate(&mut nodes, &mut conns) {
        return false;
    }
    graph["nodes"] = Value::Array(nodes);
    graph["connections"] = Value::Array(conns);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u64, kind: &str, state: Value) -> Value {
        json!({"id": id, "kind": kind, "pos": {"x": 0.0, "y": 0.0}, "fields": {}, "state": state})
    }

    #[test]
    fn flux_chain() {
        let mut g = json!({
            "nodes": [
                node(1, "flux_checkpoint", json!({"kind": "FluxCheckpoint", "data": {"model_path": "/m/flux.1-dev.syn", "quant_idx": 1}})),
                node(2, "text_view", Value::Null),
                node(3, "flux_text_encoder", Value::Null),
                node(4, "flux_empty_latent", json!({"kind": "FluxEmptyLatent", "data": {"width": 768, "height": 512}})),
                node(5, "flux_sampler", json!({"kind": "FluxSampler", "data": {"steps": 24, "guidance": 3.0, "seed": 7, "denoise": 1.0}})),
                node(6, "flux_vae_decode", Value::Null),
                node(7, "image_save", Value::Null)
            ],
            "connections": [
                conn(1, "model", 3, "model"), conn(2, "text", 3, "prompt"), conn(1, "model", 5, "model"),
                conn(3, "conditioning", 5, "conditioning"), conn(4, "latent", 5, "latent"),
                conn(1, "model", 6, "model"), conn(5, "latent", 6, "latent"), conn(6, "image", 7, "image")
            ]
        });
        assert!(migrate_graph(&mut g));
        let kinds: Vec<&str> = g["nodes"].as_array().unwrap().iter().map(|n| n["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["flow_image_model", "text_view", "flow_image_encode", "flow_image_latent", "flow_image_sample", "flow_image_decode", "image_save"]);
        assert_eq!(g["nodes"][0]["fields"]["model"]["value"], "/m/flux.1-dev.syn");
        assert_eq!(g["nodes"][0]["fields"]["quant"]["value"], 2); // mxfp8 в QUANTS
        assert_eq!(g["nodes"][4]["fields"]["seed"]["value"], 7.0);
        assert!(g["connections"].as_array().unwrap().contains(&conn(3, "cond", 5, "cond")));
        assert!(g["connections"].as_array().unwrap().contains(&conn(6, "image", 7, "image")));
        // Повторно — без изменений.
        assert!(!migrate_graph(&mut g));
    }

    #[test]
    fn h3_chain() {
        let mut g = json!({
            "nodes": [
                node(1, "h3_checkpoint", Value::Null),
                node(2, "h3_text_encoder", Value::Null),
                node(3, "h3_empty_latent_av", json!({"kind": "H3EmptyLatentAv", "data": {"width": 960, "height": 544, "duration_seconds": 3.0}})),
                node(4, "h3_keyframe", json!({"kind": "H3Keyframe", "data": {"image_path": "/a.png"}})),
                node(5, "h3_sampler", json!({"kind": "H3Sampler", "data": {"steps": 6, "cfg_scale": 1.0, "seed": 0}})),
                node(6, "h3_vae_decode", Value::Null),
                node(7, "h3_audio_decode", Value::Null),
                node(8, "h3_video_save", Value::Null),
                node(9, "flux_vae_decode", Value::Null),
                node(10, "h3_keyframe", Value::Null)
            ],
            "connections": [
                conn(1, "model", 2, "model"), conn(4, "keyframe", 2, "keyframe"), conn(1, "model", 5, "model"),
                conn(2, "conditioning", 5, "conditioning"), conn(3, "av_latent", 5, "av_latent"), conn(4, "keyframe", 5, "keyframe"),
                conn(1, "model", 6, "model"), conn(5, "video_latent", 6, "video_latent"),
                conn(1, "model", 7, "model"), conn(5, "audio_latent", 7, "audio_latent"),
                conn(6, "frames", 8, "frames"), conn(7, "audio", 8, "audio"),
                conn(9, "image", 10, "image"), conn(10, "keyframe", 2, "keyframe_last")
            ]
        });
        assert!(migrate_graph(&mut g));
        let c = g["connections"].as_array().unwrap();
        assert!(c.contains(&conn(3, "latent", 2, "latent")));
        assert!(c.contains(&conn(4, "file", 2, "first_frame")));
        assert!(c.contains(&conn(9, "image", 2, "last_frame")));
        assert!(c.contains(&conn(5, "latent", 6, "latent")));
        assert!(c.contains(&conn(6, "video", 8, "video")));
        assert!(!c.iter().any(|x| x["to_node"] == 5 && x["to_port"] == "av_latent"));
        // Ключевой кадр с проводом картинки убран.
        assert!(!g["nodes"].as_array().unwrap().iter().any(|n| n["id"] == 10));
        assert_eq!(g["nodes"][0]["fields"]["model"]["value"], "minimax-h3-fl2va");
        assert_eq!(g["nodes"][4]["fields"]["guidance"]["value"], 1.0);
    }

    #[test]
    fn ltx_chain() {
        let mut g = json!({
            "nodes": [
                node(1, "ltx_checkpoint", Value::Null),
                node(3, "ltx_text_encoder", json!({"kind": "LtxTextEncoder", "data": {"prompt": "кот"}})),
                node(5, "ltx_sampler_stage1", json!({"kind": "LtxSamplerStage1", "data": {"width": 768, "height": 448, "duration_seconds": 4.0, "fps_idx": 1, "seed": 9}})),
                node(6, "ltx_upscale", Value::Null),
                node(7, "ltx_sampler_stage2", Value::Null),
                node(8, "ltx_vae_decode", Value::Null),
                node(9, "ltx_audio_decode", Value::Null),
                node(13, "ltx_image", json!({"kind": "LtxImage", "data": {"image_path": "/a.png", "strength": 0.8, "frame_idx": 0}}))
            ],
            "connections": [
                conn(1, "model", 5, "model"), conn(3, "video_encoding", 5, "video_encoding"), conn(3, "audio_encoding", 5, "audio_encoding"),
                conn(13, "image_cond", 5, "image_cond"),
                conn(5, "audio_tokens", 7, "audio_tokens"), conn(5, "video_latent", 6, "video_latent"), conn(6, "video_latent", 7, "video_latent"),
                conn(7, "video_latent", 8, "video_latent"), conn(7, "audio_tokens", 9, "audio_tokens")
            ]
        });
        assert!(migrate_graph(&mut g));
        let c = g["connections"].as_array().unwrap();
        assert!(c.contains(&conn(6, "latent", 7, "latent")));
        assert!(!c.iter().any(|x| x["from_node"] == 5 && x["to_node"] == 7));
        assert!(c.contains(&conn(3, "cond", 5, "cond")));
        assert_eq!(c.iter().filter(|x| x["to_node"] == 5 && x["to_port"] == "cond").count(), 1);
        assert!(c.contains(&conn(7, "latent", 9, "latent")));
        let n = |id: u64| g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).unwrap().clone();
        assert_eq!(n(5)["fields"]["fps"]["value"], 1);
        assert_eq!(n(3)["fields"]["prompt"]["value"], "кот");
        assert_eq!(n(1)["fields"]["model"]["value"], "ltx-2-3-22b-distilled-1-1");
        // Кадр-условие с файлом — новая нода «Файл» перед ним.
        let file = g["nodes"].as_array().unwrap().iter().find(|n| n["kind"] == "flow_file").unwrap().clone();
        assert_eq!(file["fields"]["path"]["value"], "/a.png");
        assert!(c.contains(&conn(file["id"].as_u64().unwrap(), "file", 13, "image")));
    }

    #[test]
    fn music_chain() {
        let mut g = json!({
            "nodes": [
                node(1, "ace_step_checkpoint", json!({"kind": "AceStepCheckpoint", "data": {"dit_path": "acestep_v15_xl_turbo.syn", "lm_path": "acestep_5hz_lm_1.7b.syn", "quant_dit_idx": 1}})),
                node(2, "audio_file", Value::Null),
                node(3, "ace_step_vae_encode", json!({"kind": "AceStepVaeEncode", "data": {"chunk_seconds": 20.0}})),
                node(4, "ace_step_generate", json!({"kind": "AceStepGenerate", "data": {"mode_idx": 2, "preset": "Auto", "infer_steps": 8, "use_ar": false, "norm_mode": "none", "enable_dcw": true, "dcw_mode": "Low", "track_idx": 3}})),
                node(5, "yue2_checkpoint", Value::Null),
                node(6, "yue2_transcribe", json!({"kind": "Yue2Transcribe", "data": {"mode_idx": 1}})),
                node(7, "yue2_generate", json!({"kind": "Yue2Generate", "data": {"cot_idx": 1, "ode_steps": 16}}))
            ],
            "connections": [
                conn(1, "model", 3, "model"), conn(2, "out", 3, "audio"), conn(1, "model", 4, "model"), conn(3, "latent", 4, "src_latent"),
                conn(5, "model", 6, "model"), conn(2, "out", 6, "audio"), conn(5, "model", 7, "model"), conn(6, "score", 7, "abc")
            ]
        });
        assert!(migrate_graph(&mut g));
        let c = g["connections"].as_array().unwrap();
        assert!(c.contains(&conn(3, "latent", 4, "latent")));
        assert!(c.contains(&conn(2, "out", 3, "audio")));
        assert!(c.contains(&conn(6, "score", 7, "abc")));
        let n = |id: u64| g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).unwrap().clone();
        assert_eq!(n(1)["kind"], "flow_acestep_model");
        assert_eq!(n(1)["fields"]["model"]["value"], "acestep-v15-xl-turbo");
        assert_eq!(n(1)["fields"]["lm"]["value"], 1);
        assert_eq!(n(1)["fields"]["quant"]["value"], 0); // NVFP4 медиа не ставится
        assert_eq!(n(3)["fields"]["chunk"]["value"], 20.0);
        let f = &n(4)["fields"];
        assert_eq!(f["mode"]["value"], 2);
        assert!(f.get("steps").is_none()); // Auto — умолчания варианта
        assert_eq!(f["ar"]["value"], false);
        assert_eq!(f["norm"]["value"], 2); // off
        assert_eq!(f["dcw"]["value"], 2); // low
        assert_eq!(f["track"]["value"], 3);
        assert_eq!(n(5)["fields"]["model"]["value"], "yue2-3b");
        assert_eq!(n(6)["fields"]["mode"]["value"], 1);
        assert_eq!(n(7)["fields"]["cot"]["value"], 1);
        assert_eq!(n(7)["fields"]["steps"]["value"], 16.0);
    }

    #[test]
    fn speech_chain() {
        let mut g = json!({
            "nodes": [
                node(1, "syn_checkpoint", json!({"kind": "SynCheckpoint", "data": {"model_path": "/m/vibevoice-7b.syn"}})),
                node(2, "vibe_voice", json!({"kind": "VibeVoice", "data": {"script": "Speaker 1: hi", "cfg_value": 1.5, "ddpm_steps": 30}})),
                node(3, "audio_file", Value::Null),
                node(4, "syn_checkpoint", Value::Null),
                node(5, "llm", json!({"kind": "Llm", "data": {"system_prompt": "be brief", "think": true, "seed": 0, "temperature": 0.3}})),
                node(6, "asr_gigaam", Value::Null),
                node(7, "sortformer_diarizer", json!({"kind": "SortformerDiarizer", "data": {"threshold": 0.4, "allow_overlap": false}}))
            ],
            "connections": [
                conn(1, "model", 2, "model"), conn(3, "out", 2, "voice1"), conn(4, "model", 5, "model"),
                conn(5, "answer", 2, "script"), conn(3, "out", 6, "in"), conn(3, "out", 7, "in")
            ]
        });
        assert!(migrate_graph(&mut g));
        let ns = g["nodes"].as_array().unwrap();
        assert!(!ns.iter().any(|n| n["kind"] == "syn_checkpoint"));
        let n = |id: u64| ns.iter().find(|n| n["id"] == id).unwrap().clone();
        assert_eq!(n(2)["kind"], "flow_vibevoice");
        assert_eq!(n(2)["fields"]["model"]["value"], "/m/vibevoice-7b.syn");
        assert_eq!(n(2)["fields"]["steps"]["value"], 30.0);
        assert_eq!(n(5)["kind"], "flow_llm");
        assert!(n(5)["fields"].get("model").is_none());
        assert!(n(5)["fields"].get("seed").is_none());
        assert_eq!(n(5)["fields"]["think"]["value"], true);
        assert_eq!(n(6)["fields"]["model"]["value"], "gigaam-v3");
        assert_eq!(n(7)["fields"]["overlap"]["value"], false);
        let c = g["connections"].as_array().unwrap();
        assert!(c.contains(&conn(5, "text", 2, "script")));
        assert!(c.contains(&conn(3, "out", 6, "file")));
        assert!(c.contains(&conn(3, "out", 7, "audio")));
        assert!(!c.iter().any(|x| x["to_port"] == "model"));
    }
}
