//! Сохранение / восстановление сессии node-editor'а между запусками.
//!
//! Состояние всех открытых вкладок (граф каждой + viewport + активная)
//! персистится в `~/.config/synthos/workspace.json`. Файл переписывается
//! автосохранением (см. `install_workspace_autosave` в `lib.rs`) на любое
//! изменение: добавление/удаление ноды, drag, изменение dropdown'ов, ввод
//! текста и т. д. На старте приложения [`EditorWorkspace::new_or_restore`]
//! читает файл и восстанавливает вкладки; при отсутствии файла создаётся
//! одна пустая `Untitled`-вкладка как раньше.
//!
//! Сериализуемая модель графа полностью переиспользует `templates::model`
//! (`NodeData`, `ConnData`, `ViewportData`, `NodeStateData`) — там уже
//! заведена JSON-схема со всеми per-kind runtime-параметрами нод.
//!
//! Тяжёлые объекты (Transcriber/Pipeline/AudioPlayer/threads/channels) НЕ
//! сериализуются — пересоздаются дефолтно через `registry::default_runtime`,
//! модели грузятся лениво на первый Play (как при загрузке шаблона).

use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::PathBuf;

use syngui::signal::create_effect;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::state::NodeEditorCtx;
use super::tabs::OpenTab;
use super::types::{FieldValue, NodeInstance, NodeRuntime};
use crate::templates::convert::snapshot;
use crate::templates::model::{ConnData, NodeData, ViewportData};

/// Состояние одной вкладки на диске.
///
/// Поля симметричны `OpenTab` без runtime-объектов: `title` / `source` /
/// `nodes` / `connections` / `viewport`. `id` тоже сохраняется, чтобы
/// после restore не было коллизий с auto-increment-генератором новых id'ов.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabState {
    /// Id вкладки (из `EditorWorkspace::next_tab_id` на момент создания).
    pub id: u64,
    /// Заголовок вкладки. Untitled / имя загруженного шаблона / переименованное.
    pub title: String,
    /// Id шаблона, из которого вкладка была открыта (для UX-дедупликации
    /// `open_template` — повторный клик активирует существующую). `None` =
    /// Untitled или scratch.
    #[serde(default)]
    pub source: Option<String>,
    /// Снимок нод. NodeData уже несёт всю pos/style/enabled/fields/state информацию.
    #[serde(default)]
    pub nodes: Vec<NodeData>,
    /// Соединения.
    #[serde(default)]
    pub connections: Vec<ConnData>,
    /// Pan/zoom.
    #[serde(default)]
    pub viewport: Option<ViewportData>,
}

/// Состояние всего workspace'а node-editor'а.
///
/// `next_tab_id` сохраняется чтобы при restart'е новые вкладки получали id
/// большие, чем все восстановленные, — иначе будут коллизии в `tabs.id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceState {
    #[serde(default)]
    pub tabs: Vec<TabState>,
    /// Id активной вкладки. None если ни одна не активна (редкий кейс).
    #[serde(default)]
    pub active: Option<u64>,
    /// Auto-increment для новых вкладок после restore.
    #[serde(default = "default_next_tab_id")]
    pub next_tab_id: u64,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            next_tab_id: default_next_tab_id(),
        }
    }
}

fn default_next_tab_id() -> u64 {
    1
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Путь к файлу workspace'а: `~/.config/synthos/workspace.json`. Каталог
/// создаётся лениво при первой записи.
pub fn workspace_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".config/synthos")
        .join("workspace.json")
}

/// Прочитать сохранённый workspace. Возвращает `None` если файла нет;
/// при парсинг-ошибке логируется warning и возвращается `None` (graceful —
/// пользователь не должен видеть panic из-за битого JSON между версиями).
pub fn load() -> Option<WorkspaceState> {
    load_from(&workspace_path())
}

pub fn load_from(path: &std::path::Path) -> Option<WorkspaceState> {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "не удалось прочитать workspace.json");
            return None;
        }
    };
    let raw = strip_legacy_kinds(&raw);
    match serde_json::from_str::<WorkspaceState>(&raw) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "битый workspace.json, использую дефолт");
            None
        }
    }
}

fn strip_legacy_kinds(raw: &str) -> String {
    const LEGACY: &[&str] = &[
        "demo",
        // Старые ACE-Step ноды (заменены на checkpoint + generate). Узлы этих
        // типов из прежних workspace.json молча отбрасываются при загрузке.
        "ace_step_text_encoder",
        "ace_step_lyric_encoder",
        "ace_step_timbre_encoder",
        "ace_step_pack",
        "ace_step_vae_decode",
        "ace_step_ar_lm",
        "ace_step_sampler",
    ];
    let mut v: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return raw.to_string(),
    };
    let Some(tabs) = v.get_mut("tabs").and_then(|t| t.as_array_mut()) else {
        return raw.to_string();
    };
    for tab in tabs.iter_mut() {
        let Some(nodes) = tab.get_mut("nodes").and_then(|n| n.as_array_mut()) else { continue };
        let before = nodes.len();
        nodes.retain(|node| {
            let kind = node.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            !LEGACY.contains(&kind)
        });
        if nodes.len() != before {
            tracing::info!(removed = before - nodes.len(), "workspace: пропущены ноды устаревших типов");
        }
    }
    serde_json::to_string(&v).unwrap_or_else(|_| raw.to_string())
}

/// Сохранить workspace на диск. Каталог создаётся при отсутствии.
/// Файл пишется атомарно через temp-файл + rename, чтобы выключение во
/// время записи не оставило половинный JSON.
pub fn save(state: &WorkspaceState) -> Result<(), WorkspaceError> {
    save_to(state, &workspace_path())
}

pub fn save_to(state: &WorkspaceState, path: &std::path::Path) -> Result<(), WorkspaceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let pretty = serde_json::to_string_pretty(state)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, pretty)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Reactive subscribe — для install_workspace_autosave
//
// Чтобы автосейв-эффект перезапускался на ЛЮБОЕ изменение в графе
// (drag-pos ноды, изменение dropdown'а в runtime, ввод текста, slider'ы,
// edit_mode markdown'а), эффект должен подписаться на КАЖДЫЙ сигнал графа.
// `templates::convert::snapshot` использует `get_untracked` (потому что
// предназначен для one-shot read) — нам нужен tracked-вариант, и здесь
// он реализован отдельно от snapshot'а, чтобы не дублировать reactivity-
// логику в convert.rs.
//
// Подписка делается через `.get()` (tracked); возвращаемое значение
// игнорируется (значения собираются всё равно через snapshot вне subscribe-
// прохода). Это безопасно: signal-runtime syngui поддерживает повторное
// чтение одного и того же сигнала в одном проходе эффекта без двойной
// подписки (`SignalRuntime` хранит подписчиков в Set).
// ─────────────────────────────────────────────────────────────────────────────

/// Стабильный hash снимка вкладки. Используется как fingerprint для
/// dirty-tracking — `RwSignal<u64>` на `OpenTab` хранит fp последнего save'а,
/// каждое изменение пересчитывает fp и сравнивает.
///
/// Алгоритм: serde_json → байты → `DefaultHasher`. Pretty не используем —
/// дешевле и одинаковая JSON-форма всё равно даст один fp.
pub fn tab_fingerprint(ctx: &NodeEditorCtx) -> u64 {
    let (nodes, conns, viewport) = snapshot(ctx);
    let bytes = match serde_json::to_vec(&(nodes, conns, viewport)) {
        Ok(b) => b,
        Err(_) => return 0,
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Per-tab dirty-tracker. Подписывается на ВСЕ сигналы tab.ctx
/// через [`subscribe_tab_signals`] и пересчитывает fingerprint:
/// `dirty = current_fp != tab.last_saved_fp`. При первом проходе
/// (`last_saved_fp == 0`) — заводит исходный fp и dirty=false.
///
/// Вызывается один раз при создании каждой `OpenTab`
/// (`new_untitled` / `open_template` / `make_tab_from_state`).
pub fn install_dirty_for_tab(tab: OpenTab) {
    create_effect(move || {
        subscribe_tab_signals(&tab.ctx);
        let fp = tab_fingerprint(&tab.ctx);
        let saved = tab.last_saved_fp.get_untracked();
        if saved == 0 {
            tab.last_saved_fp.set(fp);
            if tab.dirty.get_untracked() {
                tab.dirty.set(false);
            }
            return;
        }
        let new_dirty = fp != saved;
        if tab.dirty.get_untracked() != new_dirty {
            tab.dirty.set(new_dirty);
        }
    });
}

/// Подписать текущий effect-scope на ВСЕ сигналы ctx'а одной вкладки.
/// Вызывается в начале effect'а; затем вне subscribe-прохода делается
/// `templates::convert::snapshot` для сбора актуальных значений.
pub fn subscribe_tab_signals(ctx: &NodeEditorCtx) {
    // Top-level: добавление/удаление нод, добавление/удаление связей, viewport.
    let nodes = ctx.nodes.get();
    let _ = ctx.connections.get();
    let _ = ctx.pan.get();
    let _ = ctx.zoom.get();

    for node in nodes.iter() {
        subscribe_node_signals(node);
    }
}

fn subscribe_node_signals(node: &NodeInstance) {
    let _ = node.pos.get();
    let _ = node.enabled.get();
    let _ = node.style.get();

    if let Ok(map) = node.fields.lock() {
        for v in map.values() {
            match v {
                FieldValue::Text(s) => {
                    let _ = s.get();
                }
                FieldValue::Float(s) => {
                    let _ = s.get();
                }
                FieldValue::Int(s) => {
                    let _ = s.get();
                }
                FieldValue::Bool(s) => {
                    let _ = s.get();
                }
                FieldValue::Color(s) => {
                    let _ = s.get();
                }
                FieldValue::Choice(s) => {
                    let _ = s.get();
                }
            }
        }
    }

    if let Ok(rt) = node.runtime.lock() {
        subscribe_runtime_signals(&rt);
    }
}

fn subscribe_runtime_signals(rt: &NodeRuntime) {
    match rt {
        NodeRuntime::None => {}
        NodeRuntime::H3Checkpoint {
            model_dir,
            encoder_dir,
            lora_path,
            lora_strength,
            variant_idx,
            device_idx,
            quant_dit_idx,
            quant_enc_idx,
            compute_idx,
            memory_mode_idx,
            ..
        } => {
            let _ = model_dir.get();
            let _ = encoder_dir.get();
            let _ = lora_path.get();
            let _ = lora_strength.get();
            let _ = variant_idx.get();
            let _ = device_idx.get();
            let _ = quant_dit_idx.get();
            let _ = quant_enc_idx.get();
            let _ = compute_idx.get();
            let _ = memory_mode_idx.get();
        }
        NodeRuntime::H3TextEncoder { error, loaded_name, .. } => {
            let _ = error.get();
            let _ = loaded_name.get();
        }
        NodeRuntime::H3EmptyLatentAv { width, height, duration_seconds } => {
            let _ = width.get();
            let _ = height.get();
            let _ = duration_seconds.get();
        }
        NodeRuntime::H3Keyframe { path, frame_slot_idx, resize_idx, error, .. } => {
            let _ = path.get();
            let _ = frame_slot_idx.get();
            let _ = resize_idx.get();
            let _ = error.get();
        }
        NodeRuntime::H3Sampler { steps, cfg_scale, seed, two_stage, refine_steps, error, .. } => {
            let _ = steps.get();
            let _ = cfg_scale.get();
            let _ = seed.get();
            let _ = two_stage.get();
            let _ = refine_steps.get();
            let _ = error.get();
        }
        NodeRuntime::H3VaeDecode { error, .. } => {
            let _ = error.get();
        }
        NodeRuntime::H3AudioDecode { error, .. } => {
            let _ = error.get();
        }
        NodeRuntime::H3VideoSave { path, error, saved, .. } => {
            let _ = path.get();
            let _ = error.get();
            let _ = saved.get();
        }
        NodeRuntime::AudioFile {
            loaded_path,
            load_error,
            ..
        } => {
            let _ = loaded_path.get();
            let _ = load_error.get();
        }
        NodeRuntime::AudioPlayer {
            volume,
            is_playing,
            is_paused,
            progress,
            pcm_view,
            is_streaming,
            ..
        } => {
            let _ = volume.get();
            // Транспорт-state сохранять не нужно, но подписка дёшева;
            // главное — `volume` пользователь меняет slider'ом.
            let _ = is_playing.get();
            let _ = is_paused.get();
            let _ = progress.get();
            let _ = pcm_view.get();
            let _ = is_streaming.get();
        }
        NodeRuntime::AudioRecorder { device, .. } => {
            let _ = device.get();
        }
        NodeRuntime::Gain { gain_db, .. } => {
            let _ = gain_db.get();
        }
        NodeRuntime::Filter { mode, cutoff_hz, .. } => {
            let _ = mode.get();
            let _ = cutoff_hz.get();
        }
        NodeRuntime::Reverb { mix, room, .. } => {
            let _ = mix.get();
            let _ = room.get();
        }
        NodeRuntime::SaveToFile { path, status, .. } => {
            let _ = path.get();
            let _ = status.get();
        }
        NodeRuntime::Equalizer { gains_db, .. } => {
            for s in gains_db {
                let _ = s.get();
            }
        }
        NodeRuntime::Mixer {
            n_inputs, gains_db, ..
        } => {
            let _ = n_inputs.get();
            for s in gains_db {
                let _ = s.get();
            }
        }
        NodeRuntime::MarkdownView {
            content,
            edit_mode,
            resize_mode,
            size,
            ..
        } => {
            let _ = content.get();
            let _ = edit_mode.get();
            let _ = resize_mode.get();
            let _ = size.get();
        }
        NodeRuntime::TextView {
            output_text,
            text_version,
            size,
            resize_mode,
            ..
        } => {
            let _ = output_text.get();
            let _ = text_version.get();
            let _ = size.get();
            let _ = resize_mode.get();
        }
        NodeRuntime::AsrGigaam {
            model_path,
            device_idx,
            storage_idx,
            compute_idx,
            output_text,
            running,
            error,
            loaded_name,
            text_version,
            ..
        } => {
            let _ = model_path.get();
            let _ = device_idx.get();
            let _ = storage_idx.get();
            let _ = compute_idx.get();
            let _ = output_text.get();
            let _ = running.get();
            let _ = error.get();
            let _ = loaded_name.get();
            let _ = text_version.get();
        }
        NodeRuntime::OmniVoice {
            model_path,
            device_idx,
            storage_idx,
            compute_idx,
            instruct,
            ref_text_field,
            language,
            num_step,
            guidance_scale,
            t_shift,
            speed,
            seed,
            running,
            error,
            loaded_name,
            output_version,
            ..
        } => {
            let _ = model_path.get();
            let _ = device_idx.get();
            let _ = storage_idx.get();
            let _ = compute_idx.get();
            let _ = instruct.get();
            let _ = ref_text_field.get();
            let _ = language.get();
            let _ = num_step.get();
            let _ = guidance_scale.get();
            let _ = t_shift.get();
            let _ = speed.get();
            let _ = seed.get();
            let _ = running.get();
            let _ = error.get();
            let _ = loaded_name.get();
            let _ = output_version.get();
        }
        NodeRuntime::Llm {
            model_path,
            device_idx,
            quant_idx,
            compute_idx,
            system_prompt,
            context,
            think,
            max_tokens,
            temperature,
            top_k,
            top_p,
            min_p,
            repetition_penalty,
            seed,
            running,
            error,
            loaded_name,
            output_text,
            text_version,
            ..
        } => {
            let _ = model_path.get();
            let _ = device_idx.get();
            let _ = quant_idx.get();
            let _ = compute_idx.get();
            let _ = system_prompt.get();
            let _ = context.get();
            let _ = think.get();
            let _ = max_tokens.get();
            let _ = temperature.get();
            let _ = top_k.get();
            let _ = top_p.get();
            let _ = min_p.get();
            let _ = repetition_penalty.get();
            let _ = seed.get();
            let _ = running.get();
            let _ = error.get();
            let _ = loaded_name.get();
            let _ = output_text.get();
            let _ = text_version.get();
        }
        NodeRuntime::VoxCpm2 {
            model_path,
            device_idx,
            compute_idx,
            prompt_text_field,
            cfg_value,
            n_timesteps,
            max_len,
            seed,
            running,
            error,
            loaded_name,
            output_version,
            ..
        } => {
            let _ = model_path.get();
            let _ = device_idx.get();
            let _ = compute_idx.get();
            let _ = prompt_text_field.get();
            let _ = cfg_value.get();
            let _ = n_timesteps.get();
            let _ = max_len.get();
            let _ = seed.get();
            let _ = running.get();
            let _ = error.get();
            let _ = loaded_name.get();
            let _ = output_version.get();
        }
        NodeRuntime::SortformerDiarizer {
            model_path,
            device_idx,
            storage_idx,
            compute_idx,
            threshold,
            allow_overlap,
            running,
            error,
            loaded_name,
            output_pretty,
            output_json,
            text_version,
            ..
        } => {
            let _ = model_path.get();
            let _ = device_idx.get();
            let _ = storage_idx.get();
            let _ = compute_idx.get();
            let _ = threshold.get();
            let _ = allow_overlap.get();
            let _ = running.get();
            let _ = error.get();
            let _ = loaded_name.get();
            let _ = output_pretty.get();
            let _ = output_json.get();
            let _ = text_version.get();
        }
        // ACE-Step ноды: подписки на все persist-достойные сигналы
        // (см. `templates/convert.rs::runtime_to_state`). Изменение
        // любого из них поднимает фингерпринт → workspace.json пере-
        // сохраняется. `output_version` — для bump'а после успешного Play.
        NodeRuntime::AceStepVaeEncode {
            device_idx,
            storage_idx,
            compute_idx,
            chunk_seconds,
            overlap_seconds,
            output_version,
            ..
        } => {
            let _ = device_idx.get();
            let _ = storage_idx.get();
            let _ = compute_idx.get();
            let _ = chunk_seconds.get();
            let _ = overlap_seconds.get();
            let _ = output_version.get();
        }
        NodeRuntime::FfmpegPlayer {
            current_path,
            volume,
            hwaccel_idx,
            size,
            is_playing,
            is_paused,
            progress,
            duration,
            position,
            out_version,
            ..
        } => {
            let _ = current_path.get();
            let _ = volume.get();
            let _ = hwaccel_idx.get();
            let _ = size.get();
            let _ = is_playing.get();
            let _ = is_paused.get();
            let _ = progress.get();
            let _ = duration.get();
            let _ = position.get();
            let _ = out_version.get();
        }
        NodeRuntime::LtxCheckpoint {
            model_path,
            gemma_dir,
            upscaler_path,
            lora_path,
            lora_strength,
            device_idx,
            quant_dit_idx,
            quant_enc_idx,
            compute_idx,
            ..
        } => {
            let _ = model_path.get();
            let _ = gemma_dir.get();
            let _ = upscaler_path.get();
            let _ = lora_path.get();
            let _ = lora_strength.get();
            let _ = device_idx.get();
            let _ = quant_dit_idx.get();
            let _ = quant_enc_idx.get();
            let _ = compute_idx.get();
        }
        NodeRuntime::LtxTextEncoder {
            prompt_field,
            keep_gemma,
            output_version,
            ..
        } => {
            let _ = prompt_field.get();
            let _ = keep_gemma.get();
            let _ = output_version.get();
        }
        NodeRuntime::LtxNagPrompt {
            prompt_field,
            scale,
            alpha,
            tau,
            output_version,
            ..
        } => {
            let _ = prompt_field.get();
            let _ = scale.get();
            let _ = alpha.get();
            let _ = tau.get();
            let _ = output_version.get();
        }
        NodeRuntime::LtxSamplerStage1 {
            width,
            height,
            duration_seconds,
            fps_idx,
            seed,
            output_version,
            ..
        } => {
            let _ = width.get();
            let _ = height.get();
            let _ = duration_seconds.get();
            let _ = fps_idx.get();
            let _ = seed.get();
            let _ = output_version.get();
        }
        NodeRuntime::LtxUpscale { output_version, .. } => {
            let _ = output_version.get();
        }
        NodeRuntime::LtxSamplerStage2 {
            seed,
            output_version,
            ..
        } => {
            let _ = seed.get();
            let _ = output_version.get();
        }
        NodeRuntime::LtxVaeDecode { output_version, .. } => {
            let _ = output_version.get();
        }
        NodeRuntime::LtxAudioDecode { output_version, .. } => {
            let _ = output_version.get();
        }
        NodeRuntime::LtxVideoSave { path, status, .. } => {
            let _ = path.get();
            let _ = status.get();
        }
        NodeRuntime::LtxImage { image_path, strength, frame_idx, .. } => {
            let _ = image_path.get();
            let _ = strength.get();
            let _ = frame_idx.get();
        }
        NodeRuntime::LtxVideoInput { video_path } => {
            let _ = video_path.get();
        }
        NodeRuntime::LtxRetake {
            width, height, duration_seconds, fps_idx, retake_start, retake_end, seed, output_version, ..
        } => {
            let _ = width.get();
            let _ = height.get();
            let _ = duration_seconds.get();
            let _ = fps_idx.get();
            let _ = retake_start.get();
            let _ = retake_end.get();
            let _ = seed.get();
            let _ = output_version.get();
        }
        NodeRuntime::LtxIcLora {
            width, height, duration_seconds, fps_idx, downscale, ref_strength,
            control_idx, canny_low, canny_high, depth_model_path, seed, output_version, ..
        } => {
            let _ = width.get();
            let _ = height.get();
            let _ = duration_seconds.get();
            let _ = fps_idx.get();
            let _ = downscale.get();
            let _ = ref_strength.get();
            let _ = control_idx.get();
            let _ = canny_low.get();
            let _ = canny_high.get();
            let _ = depth_model_path.get();
            let _ = seed.get();
            let _ = output_version.get();
        }
        NodeRuntime::LtxAudioInput { audio_path } => {
            let _ = audio_path.get();
        }
        NodeRuntime::LtxLipdub {
            width, height, duration_seconds, fps_idx, seed, output_version, ..
        }
        | NodeRuntime::LtxA2V {
            width, height, duration_seconds, fps_idx, seed, output_version, ..
        } => {
            let _ = width.get();
            let _ = height.get();
            let _ = duration_seconds.get();
            let _ = fps_idx.get();
            let _ = seed.get();
            let _ = output_version.get();
        }
        NodeRuntime::AceStepCheckpoint {
            models_dir,
            lm_path,
            text_encoder_path,
            dit_path,
            vae_path,
            device_idx,
            quant_dit_idx,
            quant_enc_idx,
            compute_idx,
            ..
        } => {
            let _ = models_dir.get();
            let _ = lm_path.get();
            let _ = text_encoder_path.get();
            let _ = dit_path.get();
            let _ = vae_path.get();
            let _ = device_idx.get();
            let _ = quant_dit_idx.get();
            let _ = quant_enc_idx.get();
            let _ = compute_idx.get();
        }
        NodeRuntime::AceStepGenerate {
            mode_idx,
            preset,
            duration_seconds,
            infer_steps,
            cfg_scale,
            flow_match_shift,
            seed,
            temperature,
            top_p,
            top_k,
            min_p,
            lm_cfg_scale,
            use_cot,
            use_ar,
            bpm,
            keyscale_idx,
            timesig_idx,
            norm_mode,
            enable_dcw,
            dcw_mode,
            dcw_scaler,
            dcw_high_scaler,
            dcw_wavelet,
            dcw_preset,
            retake_variance,
            retake_seed,
            repaint_start_sec,
            repaint_end_sec,
            repaint_strength,
            edit_n_min,
            edit_n_max,
            output_version,
            ..
        } => {
            let _ = mode_idx.get();
            let _ = preset.get();
            let _ = duration_seconds.get();
            let _ = infer_steps.get();
            let _ = cfg_scale.get();
            let _ = flow_match_shift.get();
            let _ = seed.get();
            let _ = temperature.get();
            let _ = top_p.get();
            let _ = top_k.get();
            let _ = min_p.get();
            let _ = lm_cfg_scale.get();
            let _ = use_cot.get();
            let _ = use_ar.get();
            let _ = bpm.get();
            let _ = keyscale_idx.get();
            let _ = timesig_idx.get();
            let _ = norm_mode.get();
            let _ = enable_dcw.get();
            let _ = dcw_mode.get();
            let _ = dcw_scaler.get();
            let _ = dcw_high_scaler.get();
            let _ = dcw_wavelet.get();
            let _ = dcw_preset.get();
            let _ = retake_variance.get();
            let _ = retake_seed.get();
            let _ = repaint_start_sec.get();
            let _ = repaint_end_sec.get();
            let _ = repaint_strength.get();
            let _ = edit_n_min.get();
            let _ = edit_n_max.get();
            let _ = output_version.get();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::node_editor::types::NodeKind;
    use crate::templates::model::{NodeStateData, GainStateData, PointData, NodeStyleData};

    #[test]
    fn empty_workspace_roundtrip() {
        let state = WorkspaceState::default();
        let json = serde_json::to_string(&state).unwrap();
        let restored: WorkspaceState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, restored);
    }

    #[test]
    fn tab_with_node_roundtrip() {
        let tab = TabState {
            id: 1,
            title: "Untitled".into(),
            source: None,
            nodes: vec![NodeData {
                id: 7,
                kind: NodeKind::Gain,
                pos: PointData { x: 120.0, y: 80.0 },
                fields: Default::default(),
                style: NodeStyleData { tint: None, shadow: true },
                enabled: true,
                state: Some(NodeStateData::Gain(GainStateData { gain_db: 6.0 })),
            }],
            connections: Vec::new(),
            viewport: None,
        };
        let state = WorkspaceState {
            tabs: vec![tab.clone()],
            active: Some(1),
            next_tab_id: 2,
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        let restored: WorkspaceState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, restored);
        assert_eq!(restored.tabs[0].nodes[0].kind, NodeKind::Gain);
    }

    /// Полный flow: save state на диск → load → from_state. Проверяет, что
    /// после restart'а у workspace'а та же конфигурация. Изолирует $HOME
    /// в tmpdir чтобы тест не топтал реальный конфиг пользователя.
    ///
    #[test]
    fn save_load_restore_full_flow() {
        use super::super::tabs::EditorWorkspace;
        use crate::templates::model::PointData;

        let tmp = std::env::temp_dir().join(format!(
            "synthos_workspace_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let ws_path = tmp.join(".config/synthos/workspace.json");
        let _ = fs::remove_dir_all(&tmp);

        let tab = TabState {
            id: 42,
            title: "My Workflow".into(),
            source: None,
            nodes: vec![NodeData {
                id: 9,
                kind: NodeKind::Gain,
                pos: PointData { x: 200.0, y: 150.0 },
                fields: Default::default(),
                style: Default::default(),
                enabled: true,
                state: Some(NodeStateData::Gain(GainStateData { gain_db: -3.0 })),
            }],
            connections: Vec::new(),
            viewport: None,
        };
        let saved = WorkspaceState {
            tabs: vec![tab],
            active: Some(42),
            next_tab_id: 43,
        };
        save_to(&saved, &ws_path).expect("save");

        // Re-read как при перезапуске app'а.
        let loaded = load_from(&ws_path).expect("load");
        assert_eq!(loaded, saved);

        let ws = EditorWorkspace::from_state(loaded);
        let tabs = ws.tabs.get_untracked();
        assert_eq!(tabs.len(), 1);
        let tab0 = &tabs[0];
        assert_eq!(tab0.id.0, 42);
        assert_eq!(tab0.title.get_untracked(), "My Workflow");

        let nodes = tab0.ctx.nodes.get_untracked();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].kind, NodeKind::Gain);
        let pos = nodes[0].pos.get_untracked();
        assert!((pos.x - 200.0).abs() < 1e-3);
        assert!((pos.y - 150.0).abs() < 1e-3);
        if let Ok(rt) = nodes[0].runtime.lock() {
            if let NodeRuntime::Gain { gain_db, .. } = &*rt {
                assert!((gain_db.get_untracked() - (-3.0)).abs() < 1e-3);
            }
        }

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_legacy_empty_file_defaults_in() {
        // Минимальный валидный JSON (с одним полем) → остальные дефолтятся.
        let json = r#"{ "tabs": [] }"#;
        let s: WorkspaceState = serde_json::from_str(json).unwrap();
        assert_eq!(s.tabs.len(), 0);
        assert_eq!(s.next_tab_id, 1);
    }
}
