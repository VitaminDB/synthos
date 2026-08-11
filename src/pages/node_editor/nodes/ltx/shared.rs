//! Shared-Model Registry для LTX-нод (паттерн `nodes::acestep::shared`).
//!
//! Кэшируются только ТЯЖЁЛЫЕ компоненты: AvDit (квантование 22B минуты),
//! Gemma-3-12B (12-24GB) и mmap-чекпойнт. VAE/AudioVae/Vocoder/Upsampler
//! грузятся в воркерах свежими — это копейки и повторяет CLI-паттерн
//! «пересоздать VAE после hard_trim» (компактная укладка весов в пуле).
//!
//! Хранение — `Weak<T>`: когда последний потребитель дропнул `Arc`,
//! следующий `get_or_load` грузит заново. Sampler Stage1/Stage2 с одним
//! хэндлом делят один AvDit; TextEncoder («держать Gemma») и NAG — одну Gemma.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};

use syngui::core::sync::Mutex;
use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_core::precision::PrecisionConfig;
use synaptix_llm_gemma3::pipeline::GemmaPipeline;
use synaptix_video_ltx23::dit::{dit_resident_bytes, AvDit};
use synaptix_video_ltx23::loader::LoraWeights;
use synaptix_video_ltx23::LtxCheckpoint;

use super::super::super::types::LtxModelHandle;
use super::{compute_from_idx, device_from_idx, quant_dit_from_idx, quant_enc_from_idx};

/// Официальная длина Gemma-контекста LTX (LTXVGemmaTokenizer): перцивер-
/// коннектор тренирован на S=1024.
pub const GEMMA_CTX: usize = 1024;

/// Hash+Eq ключ кэша из [`LtxModelHandle`] (f32 strength → bits).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LtxModelKey {
    pub model_path: PathBuf,
    pub gemma_dir: PathBuf,
    pub lora_path: Option<PathBuf>,
    pub lora_strength_bits: u32,
    pub device_idx: usize,
    pub quant_dit_idx: usize,
    pub quant_enc_idx: usize,
    pub compute_idx: usize,
}

impl LtxModelKey {
    pub fn from_handle(h: &LtxModelHandle) -> Self {
        Self {
            model_path: h.model_path.clone(),
            gemma_dir: h.gemma_dir.clone(),
            lora_path: h.lora_path.clone(),
            lora_strength_bits: h.lora_strength.to_bits(),
            device_idx: h.device_idx,
            quant_dit_idx: h.quant_dit_idx,
            quant_enc_idx: h.quant_enc_idx,
            compute_idx: h.compute_idx,
        }
    }
}

fn get_or_load<K, T>(
    cache: &'static OnceLock<Mutex<HashMap<K, Weak<T>>>>,
    key: K,
    loader: impl FnOnce(&K) -> std::result::Result<T, String>,
) -> std::result::Result<Arc<T>, String>
where
    K: std::hash::Hash + Eq + Clone + 'static,
    T: Send + Sync + 'static,
{
    let map = cache.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map
        .lock()
        .map_err(|_| "ltx shared-model cache mutex poisoned".to_string())?;
    g.retain(|_, w| w.strong_count() > 0);
    if let Some(arc) = g.get(&key).and_then(|w| w.upgrade()) {
        return Ok(arc);
    }
    let arc = Arc::new(loader(&key)?);
    g.insert(key, Arc::downgrade(&arc));
    Ok(arc)
}

static CKPT_CACHE: OnceLock<Mutex<HashMap<LtxModelKey, Weak<LtxCheckpoint>>>> = OnceLock::new();
static AVDIT_CACHE: OnceLock<Mutex<HashMap<LtxModelKey, Weak<AvDitShared>>>> = OnceLock::new();
static GEMMA_CACHE: OnceLock<Mutex<HashMap<LtxModelKey, Weak<GemmaPipeline>>>> = OnceLock::new();

/// Зарегистрировать compute-backend'ы synaptix (one-shot). CUDA — под
/// фичей `ltx-cuda` (зонтик `cuda` synthos).
pub fn ensure_kernels_registered() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        synaptix_kernels_cpu::ensure_registered();
        synaptix_kernels_cuda::ensure_registered();
    });
}

/// mmap-чекпойнт (CPU-вью; потребители делают `view_on(dev)`).
pub fn load_ckpt(handle: &LtxModelHandle) -> std::result::Result<Arc<LtxCheckpoint>, String> {
    ensure_kernels_registered();
    let key = LtxModelKey::from_handle(handle);
    let path = handle.model_path.clone();
    get_or_load(&CKPT_CACHE, key, move |_| {
        LtxCheckpoint::open(&path, Device::Cpu, DType::BF16)
            .map_err(|e| format!("LTX ckpt {}: {e}", path.display()))
    })
}

/// AvDit + ресурсы его жизненного цикла. `_pin` — pinned-зеркало ckpt для
/// dense-offload (стрим блоков из pinned ~45GB/s вместо NVMe-перечиток);
/// `_ckpt` держит mmap живым дольше гарда (SAFETY-контракт
/// `OffloadPinCacheGuard`).
pub struct AvDitShared {
    pub dit: AvDit,
    pub offload: bool,
    _pin: Option<synaptix_core::device::cuda::OffloadPinCacheGuard>,
    _ckpt: Arc<LtxCheckpoint>,
}

/// Авто-решение резидент-vs-offload (порт `synaptix-cli/video.rs`): веса DiT
/// + VAE + пиковые активации + квант-pад-буферы должны влезть в свободную
/// VRAM, иначе host-stream offload. `tv_max` = fp·hp·wp ЦЕЛЕВОЙ сетки
/// (stage2 ×2 — худший случай).
fn decide_offload(
    ckpt: &LtxCheckpoint,
    quant_dit: DType,
    compute: DType,
    dev: Device,
    tv_max: usize,
) -> std::result::Result<bool, String> {
    if quant_dit == compute {
        return Ok(true);
    }
    let Device::Cuda(ord) = dev else {
        return Ok(false);
    };
    let dit_b = dit_resident_bytes(ckpt, quant_dit, compute);
    let vae_b: usize = ckpt
        .infos()
        .filter(|(n, _, _)| n.starts_with("vae."))
        .map(|(_, _, s)| compute.bytes_for_numel(s.iter().product()))
        .sum();
    let act_b = tv_max * 4096 * 2 * 28;
    let quant_pad_b = (tv_max + 256) * (16384 * 2 + 16384 * 2 + 12288 * 2);
    let need = dit_b + vae_b + act_b + quant_pad_b + (1usize << 30);
    let (free, _total) =
        synaptix_core::device::cuda::mem_info(ord).map_err(|e| format!("mem_info: {e}"))?;
    if need > free {
        tracing::info!(
            "[ltx] DiT {quant_dit:?} резидентно ~{:.1}GB > свободно {:.1}GB → streaming-offload",
            dit_b as f64 / 1e9,
            free as f64 / 1e9,
        );
    }
    Ok(need > free)
}

/// AvDit через Weak-кэш: Stage1 и Stage2 с одним хэндлом получают ОДИН
/// инстанс (как CLI: один dit на обе стадии при равных LoRA-strength).
pub fn load_avdit(handle: &LtxModelHandle, tv_max: usize) -> std::result::Result<Arc<AvDitShared>, String> {
    ensure_kernels_registered();
    let key = LtxModelKey::from_handle(handle);
    let handle = handle.clone();
    get_or_load(&AVDIT_CACHE, key, move |_| {
        let ckpt = load_ckpt(&handle)?;
        let dev = device_from_idx(handle.device_idx);
        let compute = compute_from_idx(handle.compute_idx);
        let quant_dit = quant_dit_from_idx(handle.quant_dit_idx, compute);
        sync_and_trim(dev);
        let offload = decide_offload(&ckpt, quant_dit, compute, dev, tv_max)?;
        tracing::info!("[ltx] AvDit load: quant={quant_dit:?} compute={compute:?} offload={offload} ({dev:?})");
        let pin = if offload && quant_dit == compute && matches!(dev, Device::Cuda(_)) {
            let g = synaptix_core::device::cuda::OffloadPinCacheGuard::new_paused(
                &ckpt.shard_bytes(),
            );
            g.resume();
            Some(g)
        } else {
            None
        };
        let dit = if let Some(lp) = handle.lora_path.as_ref().filter(|_| handle.lora_strength > 0.0)
        {
            let lw = LoraWeights::open(lp, dev, handle.lora_strength)
                .map_err(|e| format!("LoRA {}: {e}", lp.display()))?;
            let ck = ckpt.view_on(dev).with_lora(Arc::new(lw));
            AvDit::load_with(&ck, dev, compute, quant_dit, offload)
                .map_err(|e| format!("AvDit(+LoRA): {e}"))?
        } else {
            AvDit::load_with(&ckpt, dev, compute, quant_dit, offload)
                .map_err(|e| format!("AvDit: {e}"))?
        };
        Ok(AvDitShared {
            dit,
            offload,
            _pin: pin,
            _ckpt: ckpt,
        })
    })
}

/// Gemma-3-12B через Weak-кэш (шеринг TextEncoder ↔ NAG). Для пути
/// «дропать после encode» вызывайте [`load_gemma_uncached`].
pub fn load_gemma(handle: &LtxModelHandle) -> std::result::Result<Arc<GemmaPipeline>, String> {
    ensure_kernels_registered();
    let key = LtxModelKey::from_handle(handle);
    let handle = handle.clone();
    get_or_load(&GEMMA_CACHE, key, move |_| load_gemma_uncached(&handle))
}

/// Загрузить Gemma БЕЗ кэша — Arc дропается воркером сразу после encode
/// (12-24GB VRAM освобождаются под DiT).
pub fn load_gemma_uncached(handle: &LtxModelHandle) -> std::result::Result<GemmaPipeline, String> {
    ensure_kernels_registered();
    let dev = device_from_idx(handle.device_idx);
    let compute = compute_from_idx(handle.compute_idx);
    let quant_enc = quant_enc_from_idx(handle.quant_enc_idx, compute);
    let prec = PrecisionConfig {
        compute,
        attn_w: quant_enc,
        mlp_w: quant_enc,
        lm_head: DType::BF16,
        embed: DType::BF16,
        kv: DType::BF16,
    };
    GemmaPipeline::load_with_precision(&handle.gemma_dir, dev, prec, Some(GEMMA_CTX))
        .map_err(|e| format!("Gemma load {}: {e}", handle.gemma_dir.display()))
}

/// VaeEncoder для i2v-conditioning (свежий, как VaeDecoder — копейки против
/// квантования DiT). Кэш не нужен: encode 1 кадра дёшев.
pub fn load_vae_encoder(
    handle: &LtxModelHandle,
) -> Result<synaptix_video_ltx23::vae::VaeEncoder, String> {
    ensure_kernels_registered();
    let ckpt = load_ckpt(handle)?;
    let dev = device_from_idx(handle.device_idx);
    synaptix_video_ltx23::vae::VaeEncoder::load(&ckpt, dev)
        .map_err(|e| format!("VAE encoder: {e}"))
}

/// image `[3,H,W]` [0,1] → conditioning-токены `[1, hp·wp, 128]` для сетки
/// (hp,wp): resize до (hp·32, wp·32), affine→[−1,1], VAE-encode, patchify.
/// Bit-faithful к CLI image→video препроцессингу.
pub fn image_cond_tokens(
    handle: &LtxModelHandle,
    image: &synaptix_core::tensor::Tensor,
    hp: usize,
    wp: usize,
) -> Result<synaptix_core::tensor::Tensor, String> {
    let dev = device_from_idx(handle.device_idx);
    let (ph, pw) = (hp * 32, wp * 32);
    let resized = synaptix_io::image::resize_bilinear(image, ph, pw)
        .map_err(|e| format!("resize: {e}"))?;
    let img = resized
        .contiguous()
        .and_then(|t| t.affine(2.0, -1.0))
        .and_then(|t| t.contiguous())
        .and_then(|t| t.reshape(vec![1, 3, 1, ph, pw]))
        .map_err(|e| format!("image prep: {e}"))?;
    let encoder = load_vae_encoder(handle)?;
    let latent = encoder.encode(&img).map_err(|e| format!("VAE encode: {e}"))?;
    let _ = dev;
    synaptix_video_ltx23::pipeline::frame_latent_to_tokens(&latent)
        .map_err(|e| format!("frame_latent_to_tokens: {e}"))
}

/// Декодировать видео `path` в `[1,3,F,H,W]` [−1,1] (ffmpeg → PNG-кадры в
/// temp → load_image). `n` — макс. кадров (out_frame_count). Порт CLI
/// `load_video_frames`.
pub fn load_video_frames(
    path: &std::path::Path,
    ph: usize,
    pw: usize,
    n: usize,
    dev: Device,
) -> Result<synaptix_core::tensor::Tensor, String> {
    use std::process::Command;
    let dir = std::env::temp_dir().join("synthos_ltx_vin");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
    let status = Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(path)
        .args(["-vf", &format!("scale={pw}:{ph}")])
        .arg(dir.join("f%05d.png"))
        .status()
        .map_err(|e| format!("ffmpeg запуск: {e}"))?;
    if !status.success() {
        return Err(format!("ffmpeg decode {path:?}: {status}"));
    }
    let mut frames: Vec<synaptix_core::tensor::Tensor> = Vec::with_capacity(n);
    for i in 1..=n {
        let p = dir.join(format!("f{i:05}.png"));
        if !p.exists() {
            break;
        }
        let img = synaptix_io::image::load_image(&p, dev).map_err(|e| format!("кадр {i}: {e}"))?;
        let fr = img
            .contiguous()
            .and_then(|t| t.affine(2.0, -1.0))
            .and_then(|t| t.contiguous())
            .and_then(|t| t.reshape(vec![1, 3, 1, ph, pw]))
            .map_err(|e| format!("кадр {i} prep: {e}"))?;
        frames.push(fr);
    }
    if frames.is_empty() {
        return Err("видео не дало кадров".into());
    }
    let refs: Vec<&synaptix_core::tensor::Tensor> = frames.iter().collect();
    synaptix_core::tensor::Tensor::cat(&refs, 2)
        .and_then(|t| t.contiguous())
        .map_err(|e| format!("cat кадров: {e}"))
}

/// ref-видео `[1,3,F,Hr,Wr]` (уже [−1,1] от [`load_video_frames`]) →
/// control-токены `[1, Fr·Hr·Wr, 128]` + размер `(fpr,hpr,wpr)` для IC-LoRA
/// append: VAE encode + patchify.
pub fn ref_latent_tokens(
    handle: &LtxModelHandle,
    ref_frames: &synaptix_core::tensor::Tensor,
) -> Result<(synaptix_core::tensor::Tensor, usize, usize, usize), String> {
    let encoder = load_vae_encoder(handle)?;
    let latent = encoder.encode(ref_frames).map_err(|e| format!("VAE encode ref: {e}"))?;
    let d = latent.dims();
    let (fpr, hpr, wpr) = (d[2], d[3], d[4]);
    let tokens = latent
        .reshape(vec![1, 128, fpr * hpr * wpr])
        .and_then(|t| t.transpose(1, 2))
        .and_then(|t| t.contiguous())
        .map_err(|e| format!("ref tokens: {e}"))?;
    Ok((tokens, fpr, hpr, wpr))
}

/// Декодировать аудио `path` в 16 kHz mono f32 (ffmpeg). Для audio-VAE encode
/// (lipdub reference). Порт CLI `load_audio_16k`.
pub fn load_audio_16k(path: &std::path::Path) -> Result<Vec<f32>, String> {
    use std::process::Command;
    let out = Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(path)
        .args(["-ar", "16000", "-ac", "1", "-f", "f32le", "-"])
        .output()
        .map_err(|e| format!("ffmpeg запуск: {e}"))?;
    if !out.status.success() {
        return Err(format!("ffmpeg audio decode {path:?}: {}", out.status));
    }
    Ok(out
        .stdout
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// Речь `path` → ref-аудио-токены для lipdub: 16k mono → log-mel → audio-VAE
/// encode.
pub fn audio_ref_tokens(
    handle: &LtxModelHandle,
    path: &std::path::Path,
) -> Result<synaptix_core::tensor::Tensor, String> {
    let dev = device_from_idx(handle.device_idx);
    let ckpt = load_ckpt(handle)?;
    let samples = load_audio_16k(path)?;
    let mel = synaptix_video_ltx23::audio_vae::ltx_log_mel(&[samples], dev)
        .map_err(|e| format!("mel: {e}"))?;
    let aenc = synaptix_video_ltx23::audio_vae::AudioVaeEncoder::load(&ckpt, dev)
        .map_err(|e| format!("audio encoder: {e}"))?;
    aenc.encode(&mel).map_err(|e| format!("audio encode: {e}"))
}

/// Речь/звук `path` → audio-латент `[1,fa,128]` для a2v (frozen-conditioning):
/// 16k→log-mel→audio-VAE encode, затем обрезка/паддинг по времени до `fa`
/// (audio_token_count целевого видео). Короче `fa` → дополняется повтором
/// последнего токена.
pub fn audio_input_latent(
    handle: &LtxModelHandle,
    path: &std::path::Path,
    fa: usize,
) -> Result<synaptix_core::tensor::Tensor, String> {
    let enc = audio_ref_tokens(handle, path)?; // [1,Far,128]
    let far = enc.dims()[1];
    if far == fa {
        return Ok(enc);
    }
    if far >= fa {
        enc.narrow(1, 0, fa)
            .and_then(|t| t.contiguous())
            .map_err(|e| format!("audio crop: {e}"))
    } else {
        let last = enc
            .narrow(1, far - 1, 1)
            .map_err(|e| format!("audio last: {e}"))?;
        let mut parts = vec![enc];
        for _ in far..fa {
            parts.push(last.clone());
        }
        let refs: Vec<&synaptix_core::tensor::Tensor> = parts.iter().collect();
        synaptix_core::tensor::Tensor::cat(&refs, 1)
            .and_then(|t| t.contiguous())
            .map_err(|e| format!("audio pad: {e}"))
    }
}

/// Загрузить spatial-upscaler ×2 (mean/std статистики берутся из чекпойнта).
pub fn load_upsampler(
    handle: &LtxModelHandle,
) -> Result<synaptix_video_ltx23::upscaler::Upsampler, String> {
    let up_path = handle
        .upscaler_path
        .as_ref()
        .ok_or("в Checkpoint-ноде не выбран spatial-upscaler")?;
    let dev = device_from_idx(handle.device_idx);
    let ckpt = load_ckpt(handle)?;
    let ckpt_gpu = ckpt.view_on(dev);
    let mean = ckpt_gpu
        .get_raw("vae.per_channel_statistics.mean-of-means")
        .map_err(|e| format!("vae mean: {e}"))?;
    let std = ckpt_gpu
        .get_raw("vae.per_channel_statistics.std-of-means")
        .map_err(|e| format!("vae std: {e}"))?;
    synaptix_video_ltx23::upscaler::Upsampler::load(up_path, &mean, &std, dev)
        .map_err(|e| format!("upscaler: {e}"))
}

/// Canny-control: кадры `[1,3,F,H,W]` ([−1,1]) → контурный сигнал той же формы.
/// Порт CLI `apply_canny_frames`.
pub fn apply_canny_frames(
    frames: &synaptix_core::tensor::Tensor,
    low: f32,
    high: f32,
) -> Result<synaptix_core::tensor::Tensor, String> {
    let (f, h, w) = (frames.dims()[2], frames.dims()[3], frames.dims()[4]);
    let mut out = Vec::with_capacity(f);
    for fi in 0..f {
        let fr = frames
            .narrow(2, fi, 1)
            .and_then(|t| t.contiguous())
            .and_then(|t| t.reshape(vec![3, h, w]))
            .and_then(|t| t.affine(0.5, 0.5))
            .map_err(|e| format!("canny кадр {fi}: {e}"))?;
        let edges = synaptix_io::image::canny_rgb(&fr, low, high).map_err(|e| format!("canny: {e}"))?;
        out.push(
            edges
                .affine(2.0, -1.0)
                .and_then(|t| t.reshape(vec![1, 3, 1, h, w]))
                .map_err(|e| format!("canny кадр {fi} prep: {e}"))?,
        );
    }
    let refs: Vec<&synaptix_core::tensor::Tensor> = out.iter().collect();
    synaptix_core::tensor::Tensor::cat(&refs, 2)
        .and_then(|t| t.contiguous())
        .map_err(|e| format!("canny cat: {e}"))
}

/// Depth-control: кадры `[1,3,F,H,W]` ([−1,1]) → depth-map control-сигнал
/// (Depth Anything V2). Порт CLI `apply_depth_frames`.
pub fn apply_depth_frames(
    frames: &synaptix_core::tensor::Tensor,
    model_dir: &std::path::Path,
    dev: Device,
) -> Result<synaptix_core::tensor::Tensor, String> {
    let m = synaptix_depth_anything::DepthAnything::load(model_dir, dev)
        .map_err(|e| format!("depth model: {e}"))?;
    let (f, h, w) = (frames.dims()[2], frames.dims()[3], frames.dims()[4]);
    let mut out = Vec::with_capacity(f);
    for fi in 0..f {
        let fr = frames
            .narrow(2, fi, 1)
            .and_then(|t| t.contiguous())
            .and_then(|t| t.reshape(vec![3, h, w]))
            .and_then(|t| t.affine(0.5, 0.5))
            .map_err(|e| format!("depth кадр {fi}: {e}"))?;
        let d = m.depth_rgb(&fr).map_err(|e| format!("depth: {e}"))?;
        out.push(
            d.affine(2.0, -1.0)
                .and_then(|t| t.reshape(vec![1, 3, 1, h, w]))
                .map_err(|e| format!("depth кадр {fi} prep: {e}"))?,
        );
    }
    let refs: Vec<&synaptix_core::tensor::Tensor> = out.iter().collect();
    synaptix_core::tensor::Tensor::cat(&refs, 2)
        .and_then(|t| t.contiguous())
        .map_err(|e| format!("depth cat: {e}"))
}

/// Видео `path` на сетке (hp,wp) → ref-токены `[1,Fr·Hr·Wr,128]` + pixel-позиции
/// для lipdub-append.
pub fn video_ref_tokens(
    handle: &LtxModelHandle,
    path: &std::path::Path,
    hp: usize,
    wp: usize,
    out_frames: usize,
    fps: f64,
) -> Result<(synaptix_core::tensor::Tensor, Vec<f64>), String> {
    let dev = device_from_idx(handle.device_idx);
    let frames = load_video_frames(path, hp * 32, wp * 32, out_frames, dev)?;
    let (tokens, fpr, hpr, wpr) = ref_latent_tokens(handle, &frames)?;
    let pos = synaptix_video_ltx23::pipeline::pixel_coords(fpr, hpr, wpr, fps);
    Ok((tokens, pos))
}

/// Синхронизировать GPU и вернуть пул драйверу (между тяжёлыми стадиями).
pub fn sync_and_trim(dev: Device) {
    if let Device::Cuda(o) = dev {
        let _ = synaptix_core::device::cuda::synchronize_all(o);
        let _ = synaptix_core::memory::cuda_pool::hard_trim_cuda_mempool_device(o);
    }
}

/// Strong-hold AvDit между Sampler Stage1 → Upscale → Stage2 (иначе Weak-кэш
/// дропнул бы DiT после Stage1-воркера и Stage2 квантовал бы 22B заново).
/// Decode/Save-ноды снимают hold перед VAE (CLI-паттерн «дроп DiT до decode»).
/// TODO: per-GPU hold при мульти-GPU.
static AVDIT_HOLD: OnceLock<Mutex<Option<Arc<AvDitShared>>>> = OnceLock::new();

pub fn hold_avdit(dit: Arc<AvDitShared>) {
    if let Ok(mut g) = AVDIT_HOLD.get_or_init(|| Mutex::new(None)).lock() {
        *g = Some(dit);
    }
}

pub fn release_avdit_hold() {
    if let Some(m) = AVDIT_HOLD.get() {
        if let Ok(mut g) = m.lock() {
            *g = None;
        }
    }
}
