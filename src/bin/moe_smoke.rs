//! MoE-слой Qwen4Exp на настоящих весах, без GUI.
//!
//! ```sh
//! cargo run --release --bin moe_smoke -- <bundle.syn> <каталог с safetensors> [токенов]
//! ```
//!
//! Считает один и тот же MoE-блок двумя путями — из квантованного `.syn`
//! (стопки экспертов читаются готовыми парами `packed`/`scales`) и из
//! исходных плотных весов — и сверяет выходы. Заодно печатает время прохода
//! для decode (1 токен) и prefill.

use std::path::PathBuf;
use std::time::Instant;

use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_core::tensor::quant::QuantWeight;
use synaptix_core::tensor::Tensor;
use synaptix_io::weights::safetensors::SafetensorsLoader;
use synaptix_io::weights::syn_bundle::SynBundleLoader;
use synaptix_io::weights::WeightLoader;
use synaptix_llm_common::model::ModelError;
use synaptix_llm_common::moe::{MoeConfig, MoeFfn};
use synaptix_llm_common::weights::WeightSource;

const PREFIX: &str = "model.language_model.layers.0.mlp";
const HIDDEN: usize = 2560;

/// Веса из собранного `.syn`: квантованные стопки экспертов приходят
/// готовыми.
struct BundleSource(SynBundleLoader);

impl WeightSource for BundleSource {
    fn tensor(&self, key: &str, device: Device, dtype: DType) -> Result<Tensor, ModelError> {
        self.0
            .load_to(key, device, dtype)
            .map_err(|e| ModelError::Load(format!("{key}: {e}")))
    }

    fn contains(&self, key: &str) -> bool {
        self.0.names().contains(&key)
    }

    fn quant(&self, key: &str, device: Device) -> Option<Result<QuantWeight, ModelError>> {
        Some(
            self.0
                .load_quant(key, device)?
                .map_err(|e| ModelError::Load(format!("{key}: {e}"))),
        )
    }

    fn quant_stack(
        &self,
        key: &str,
        device: Device,
    ) -> Option<Result<Vec<QuantWeight>, ModelError>> {
        Some(
            self.0
                .load_quant_stack(key, device)?
                .map_err(|e| ModelError::Load(format!("{key}: {e}"))),
        )
    }
}

/// Исходные плотные веса — эталон.
struct DenseSource(SafetensorsLoader);

impl WeightSource for DenseSource {
    fn tensor(&self, key: &str, device: Device, dtype: DType) -> Result<Tensor, ModelError> {
        self.0
            .load_to(key, device, dtype)
            .map_err(|e| ModelError::Load(format!("{key}: {e}")))
    }

    fn contains(&self, key: &str) -> bool {
        self.0.names().contains(&key)
    }
}

fn shards(dir: &PathBuf) -> anyhow::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "safetensors"))
        .collect();
    out.sort();
    anyhow::ensure!(!out.is_empty(), "в {} нет safetensors", dir.display());
    Ok(out)
}

/// Активации после нормы — небольшие значения около нуля.
fn activations(t: usize, device: Device) -> anyhow::Result<Tensor> {
    let mut s = 0x51ed_2701u64;
    let data: Vec<f32> = (0..t * HIDDEN)
        .map(|_| {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (((s >> 33) as f32 / (1u64 << 31) as f32) - 0.5) * 0.06
        })
        .collect();
    Ok(Tensor::from_vec::<_, f32>(data, (t, HIDDEN), device)?.to_dtype(DType::F16)?)
}

fn host(t: &Tensor) -> anyhow::Result<Vec<f32>> {
    Ok(t.to_device(Device::Cpu)?
        .to_dtype(DType::F32)?
        .flatten_all()?
        .to_vec1::<f32>()?)
}

fn l2_rel(got: &[f32], want: &[f32]) -> f32 {
    let num: f64 = got.iter().zip(want).map(|(a, b)| ((a - b) as f64).powi(2)).sum();
    let den: f64 = want.iter().map(|v| (*v as f64).powi(2)).sum();
    (num / den.max(1e-12)).sqrt() as f32
}

/// Форма MoE берётся из `config.json` самого бандла — так же, как её возьмёт
/// модель, а не из констант в коде.
fn config_from_bundle(path: &PathBuf) -> anyhow::Result<MoeConfig> {
    let bundle = synaptix_bundle::Bundle::open(path)?;
    let raw = bundle.read_file("config.json")?;
    let json: serde_json::Value = serde_json::from_slice(&raw)?;
    let text = json.get("text_config").unwrap_or(&json);
    let num = |key: &str| -> anyhow::Result<usize> {
        text.get(key)
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .ok_or_else(|| anyhow::anyhow!("config.json: нет `{key}`"))
    };
    let hidden = num("hidden_size")?;
    anyhow::ensure!(hidden == HIDDEN, "смок написан под hidden_size={HIDDEN}, а в конфиге {hidden}");
    Ok(MoeConfig {
        hidden_size: hidden,
        moe_intermediate_size: num("moe_intermediate_size")?,
        num_experts: num("num_experts")?,
        num_experts_per_tok: num("num_experts_per_tok")?,
        shared_intermediate_size: num("shared_expert_intermediate_size").unwrap_or(0),
        // Значение по умолчанию у класса конфига — `true`, оно же у Qwen3-MoE.
        norm_topk_prob: text
            .get("norm_topk_prob")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        chunk: MoeConfig::qwen4_exp(hidden).chunk,
        skip_below: 0.0,
        router_key: None,
        per_expert_scale: None,
        activation: synaptix_llm_common::Activation::Silu,
    })
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let bundle: PathBuf = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: moe_smoke <bundle.syn> <src dir> [токенов]"))?
        .into();
    let src_dir: PathBuf = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("не задан каталог с исходными safetensors"))?
        .into();
    let tokens: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(16);

    synaptix_kernels_cpu::ensure_registered();
    synaptix_kernels_cuda::ensure_registered();
    let device = Device::Cuda(0);
    anyhow::ensure!(
        synaptix_core::device::cuda::get(0).is_ok(),
        "нужна CUDA: квант-эксперты считаются на GPU"
    );

    let cfg = config_from_bundle(&bundle)?;
    println!(
        "MoE: {} экспертов, {} на токен, inter={}, shared={}",
        cfg.num_experts, cfg.num_experts_per_tok, cfg.moe_intermediate_size,
        cfg.shared_intermediate_size
    );

    let started = Instant::now();
    let quantized = MoeFfn::load(
        &BundleSource(SynBundleLoader::open(&bundle)?.with_device(device)),
        PREFIX,
        cfg.clone(),
        device,
        DType::F16,
        DType::MXFP8,
    )?;
    println!("из бандла загружено за {:.1} с", started.elapsed().as_secs_f64());

    let started = Instant::now();
    let dense = MoeFfn::load(
        &DenseSource(SafetensorsLoader::open_sharded(&shards(&src_dir)?)?),
        PREFIX,
        cfg.clone(),
        device,
        DType::F16,
        DType::F16, // плотный путь: без квантования
    )?;
    println!("плотный эталон загружен за {:.1} с", started.elapsed().as_secs_f64());

    let x = activations(tokens, device)?;
    let want = host(&dense.forward(&x)?)?;
    let got = host(&quantized.forward(&x)?)?;
    let err = l2_rel(&got, &want);
    println!("prefill {tokens} токенов: L2={err:.4}");
    anyhow::ensure!(err < 0.15, "квантованный MoE разошёлся с плотным: L2={err}");

    // Единственный токен — путь decode.
    let x1 = activations(1, device)?;
    let want1 = host(&dense.forward(&x1)?)?;
    let got1 = host(&quantized.forward(&x1)?)?;
    let err1 = l2_rel(&got1, &want1);
    println!("decode 1 токен: L2={err1:.4}");
    anyhow::ensure!(err1 < 0.15, "decode разошёлся: L2={err1}");

    // Время: первый проход прогревает ядра, поэтому меряем со второго.
    for (name, t) in [("decode", 1usize), ("prefill", tokens)] {
        let x = activations(t, device)?;
        let _ = host(&quantized.forward(&x)?)?;
        let started = Instant::now();
        let runs = if t == 1 { 20 } else { 5 };
        for _ in 0..runs {
            let _ = host(&quantized.forward(&x)?)?;
        }
        let per = started.elapsed().as_secs_f64() / runs as f64;
        println!("{name} ({t} ток.): {:.2} мс/слой", per * 1000.0);
    }

    println!("проверка пройдена");
    Ok(())
}
