//! Проверка квантующей упаковки на настоящей модели, без GUI.
//!
//! ```sh
//! cargo run --release --bin quant_pack_smoke -- <каталог модели> <out.syn> [nvfp4|mxfp8]
//! ```
//!
//! Собирает бандл тем же кодом, что и страница пакетов
//! ([`synthos::pages::syn_explorer::quant_pack`]), затем открывает результат
//! и сверяет: возможность формата объявлена, манифест на месте, у каждого
//! квантованного тензора есть пара `.qpacked`/`.qscales` обещанного размера,
//! а суммарный вес совпал с оценкой, которую UI показывал до упаковки.

use std::path::PathBuf;

use synaptix_bundle::inspect::{self, QuantKind};
use synaptix_bundle::pack_plan::PackPlan;
use synaptix_bundle::{Bundle, BundleBuilder, FileTag};
use synthos::pages::syn_explorer::quant_pack::{self, QuantDecision, QuantizingStream};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let src: PathBuf = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: quant_pack_smoke <src> <out.syn> [nvfp4|mxfp8]"))?
        .into();
    let out: PathBuf = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("не задан выходной .syn"))?
        .into();
    let kind = match args.next().as_deref() {
        Some("mxfp8") => QuantKind::Mxfp8,
        _ => QuantKind::Nvfp4,
    };

    synaptix_kernels_cpu::ensure_registered();
    synaptix_kernels_cuda::ensure_registered();
    anyhow::ensure!(quant_pack::cuda_available(), "нужна CUDA: квант считается на GPU");

    let plan = PackPlan::scan(&src)?;
    let comp = plan
        .components
        .iter()
        .find(|c| c.enabled)
        .ok_or_else(|| anyhow::anyhow!("в плане нет включённых компонентов"))?;
    println!(
        "источник: {} · компонент {} · шардов {}",
        plan.root.display(),
        comp.name,
        comp.paths.len()
    );

    // Квантуем то же, что предлагает UI: внимание, MLP и голову.
    let decision = QuantDecision {
        hint: format!("{} {} {}", comp.name, plan.meta.purpose, plan.meta.id),
        by_role: vec![
            (inspect::LayerRole::Attention, kind),
            (inspect::LayerRole::Mlp, kind),
            (inspect::LayerRole::LmHead, kind),
        ],
        by_group: Vec::new(),
    };

    let decide = {
        let d = decision.clone();
        move |name: &str, shape: &[usize]| d.for_tensor(name, shape)
    };
    let stream = QuantizingStream::new(
        &comp.paths,
        &decide,
        synaptix_core::device::Device::Cuda(0),
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    let quantized = stream.quantized_count();
    let manifest = stream.manifest_json().map_err(|e| anyhow::anyhow!(e))?;
    println!("квантуется тензоров: {quantized}");
    anyhow::ensure!(quantized > 0, "ни один тензор не подошёл под {kind:?}");

    let mut b = BundleBuilder::new(&plan.meta.id, &plan.meta.version);
    if !plan.meta.arch.is_empty() {
        b = b.arch(&plan.meta.arch);
    }
    b = b
        .add_tensor_stream(&comp.name, quant_pack::boxed(stream))
        .add_file_bytes(quant_pack::MANIFEST_NAME, manifest, FileTag::Inference)?
        .require_capability(quant_pack::CAP_QUANT);
    for f in plan.aux.iter().filter(|f| f.enabled) {
        b = b.add_file_path(&f.rel, &f.path, f.tag)?;
    }

    let started = std::time::Instant::now();
    b.write(&out)?;
    let elapsed = started.elapsed();
    let size = std::fs::metadata(&out)?.len();
    println!("записано {} за {:.1} с", human(size), elapsed.as_secs_f64());

    verify(&out, &comp.paths, kind, quantized)?;
    verify_readback(&out, &comp.paths, kind)?;
    println!("проверка пройдена");
    Ok(())
}

fn verify(
    out: &PathBuf,
    shards: &[PathBuf],
    kind: QuantKind,
    expect_quantized: usize,
) -> anyhow::Result<()> {
    let bundle = Bundle::open(out)?;
    anyhow::ensure!(
        bundle.meta().required_caps.iter().any(|c| c == quant_pack::CAP_QUANT),
        "бандл не объявил возможность {}",
        quant_pack::CAP_QUANT
    );
    let raw = bundle.read_file(quant_pack::MANIFEST_NAME)?;
    let manifest: quant_pack::QuantManifest = serde_json::from_slice(&raw)?;
    anyhow::ensure!(
        manifest.tensors.len() == expect_quantized,
        "в манифесте {} записей, а квантовали {expect_quantized}",
        manifest.tensors.len()
    );

    let component = bundle
        .cdir()
        .entries
        .iter()
        .find(|e| e.is_alive() && matches!(e.kind_typed(), synaptix_bundle::ChunkType::Tensors))
        .map(|e| e.name.trim_start_matches("tensors:").to_string())
        .ok_or_else(|| anyhow::anyhow!("в бандле нет tensors-чанка"))?;
    let packed_stream = bundle.tensors_slice_named(&component)?;
    let inside = inspect::read_header_slice(packed_stream)?;
    let by_name: std::collections::HashMap<&str, &inspect::TensorInfo> =
        inside.iter().map(|t| (t.name.as_str(), t)).collect();

    // Каждый квантованный тензор представлен парой блобов ожидаемого размера,
    // а исходного имени в бандле больше нет.
    for (name, entry) in &manifest.tensors {
        anyhow::ensure!(!by_name.contains_key(name.as_str()), "остался плотный `{name}`");
        let packed = by_name
            .get(format!("{name}{}", manifest.packed_suffix).as_str())
            .ok_or_else(|| anyhow::anyhow!("нет упакованных весов для `{name}`"))?;
        let scales = by_name
            .get(format!("{name}{}", manifest.scales_suffix).as_str())
            .ok_or_else(|| anyhow::anyhow!("нет масштабов для `{name}`"))?;
        let expected = inspect::quantized_bytes(&entry.shape, kind)
            .ok_or_else(|| anyhow::anyhow!("`{name}`: форма не квантуема"))?;
        anyhow::ensure!(
            packed.bytes + scales.bytes == expected,
            "`{name}`: {} + {} байт, ожидалось {expected}",
            packed.bytes,
            scales.bytes
        );
    }

    // Не квантованные тензоры доехали как были.
    let mut source: Vec<inspect::TensorInfo> = Vec::new();
    for s in shards {
        source.append(&mut inspect::read_header_file(s)?);
    }
    for t in &source {
        if manifest.tensors.contains_key(&t.name) {
            continue;
        }
        let got = by_name
            .get(t.name.as_str())
            .ok_or_else(|| anyhow::anyhow!("пропал плотный тензор `{}`", t.name))?;
        anyhow::ensure!(
            got.bytes == t.bytes && got.shape == t.shape && got.dtype == t.dtype,
            "`{}`: содержимое изменилось",
            t.name
        );
    }
    println!(
        "в бандле {} тензоров: {} квантованных пар + {} плотных",
        inside.len(),
        manifest.tensors.len(),
        source.len() - manifest.tensors.len()
    );
    Ok(())
}

/// Главная проверка: вес, прочитанный из бандла, обязан быть бит в бит
/// тем же, что движок посчитал бы сам из плотных весов. Если это так,
/// инференс на квантованном бандле неотличим от «загрузить плотное и
/// проквантовать на лету» — только без чтения гигабайтов и без работы GPU
/// на старте.
fn verify_readback(out: &PathBuf, shards: &[PathBuf], kind: QuantKind) -> anyhow::Result<()> {
    use synaptix_core::device::Device;
    use synaptix_core::dtype::DType;
    use synaptix_io::weights::syn_bundle::SynBundleLoader;
    use synaptix_io::weights::WeightLoader;

    let device = Device::Cuda(0);
    let bundle = SynBundleLoader::open(out)?.with_device(device);
    let manifest = bundle
        .quant_manifest()
        .ok_or_else(|| anyhow::anyhow!("в бандле нет манифеста квантования"))?
        .clone();
    let source = synaptix_io::weights::safetensors::SafetensorsLoader::open_sharded(shards)?;

    let mut checked = 0usize;
    let mut stacks = 0usize;
    for (name, entry) in &manifest.tensors {
        let (slices, n, k) = entry
            .dims()
            .ok_or_else(|| anyhow::anyhow!("`{name}`: форма {:?} не матрица", entry.shape))?;
        // Стопка экспертов приходит по одному весу на эксперта; обычная
        // матрица — стопкой из одного, поэтому ветвление не нужно.
        let from_bundle = bundle
            .load_quant_stack(name, device)
            .ok_or_else(|| anyhow::anyhow!("`{name}`: читатель не увидел квант"))??;
        anyhow::ensure!(
            from_bundle.len() == slices,
            "`{name}`: прочитано {} матриц из {slices}",
            from_bundle.len()
        );
        if slices > 1 {
            stacks += 1;
        }

        // Эталон: тот же путь, которым идёт обычная загрузка плотного бандла.
        let dense = source.load_to(name, device, DType::F16)?;
        for (i, got) in from_bundle.iter().enumerate() {
            anyhow::ensure!(
                (got.n(), got.k()) == (n, k),
                "`{name}` срез {i}: форма {}×{} вместо {n}×{k}",
                got.n(),
                got.k()
            );
            let slice = if slices == 1 { dense.clone() } else { dense.narrow(0, i, 1)? };
            let slice = slice.contiguous()?.reshape((n, k))?;
            let reference = match kind {
                QuantKind::Nvfp4 => slice.quantize_to_nvfp4(),
                QuantKind::Mxfp8 => slice.quantize_to_mxfp8(),
            }?;

            let (a_packed, a_scales) = host_bytes(got)?;
            let (b_packed, b_scales) = host_bytes(&reference)?;
            anyhow::ensure!(
                a_packed == b_packed,
                "`{name}` срез {i}: упакованные веса из бандла разошлись с эталоном ({} vs {} байт)",
                a_packed.len(),
                b_packed.len()
            );
            anyhow::ensure!(
                a_scales == b_scales,
                "`{name}` срез {i}: масштабы разошлись с эталоном"
            );
            checked += 1;
        }
    }
    println!(
        "сверено с эталоном: {checked} матриц ({} тензоров, из них стопок экспертов {stacks}), расхождений нет",
        manifest.tensors.len()
    );
    Ok(())
}

fn host_bytes(
    qw: &synaptix_core::tensor::quant::QuantWeight,
) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let cpu = qw.to_device(synaptix_core::device::Device::Cpu)?;
    let packed = cpu
        .packed_arc()
        .ok_or_else(|| anyhow::anyhow!("упакованные веса освобождены"))?;
    let packed = packed
        .as_cpu()
        .ok_or_else(|| anyhow::anyhow!("упакованные веса не на хосте"))?
        .as_bytes()
        .to_vec();
    let scales = cpu
        .scales()
        .as_cpu()
        .ok_or_else(|| anyhow::anyhow!("масштабы не на хосте"))?
        .as_bytes()
        .to_vec();
    Ok((packed, scales))
}

fn human(n: u64) -> String {
    const KB: f64 = 1024.0;
    let n = n as f64;
    if n >= KB * KB * KB {
        format!("{:.2} GB", n / (KB * KB * KB))
    } else if n >= KB * KB {
        format!("{:.1} MB", n / (KB * KB))
    } else {
        format!("{:.0} KB", n / KB)
    }
}
