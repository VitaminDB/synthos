//! Квантование весов в момент упаковки бандла.
//!
//! Обычная упаковка копирует safetensors байт в байт. Здесь тот же поток
//! подменяется на квантующий: выбранные тензоры считаются на GPU в NVFP4 или
//! MXFP8 и ложатся в бандл уже упакованными. Модель на диске становится
//! втрое-вчетверо меньше, и при загрузке её не нужно квантовать заново —
//! веса читаются из mmap готовыми.
//!
//! Раскладка (`syn-quant-v1`), одинаковая для писателя и читателя:
//!
//! * `<имя>.qpacked` — упакованные веса, `U8`. Форма повторяет исходную с
//!   уполовиненной последней осью для NVFP4 (`[N, K/2]`) и без изменений для
//!   MXFP8 (`[N, K]`). Стопка экспертов `[E, N, K]` квантуется послойно и
//!   сохраняет ведущую ось.
//! * `<имя>.qscales` — блочные масштабы, `U8`, одномерный блоб: их раскладка
//!   задана ядром и разбору снаружи не подлежит.
//! * `quant_manifest.json` — какой тензор каким форматом упакован и какой
//!   формы он был. Без манифеста читатель не отличит квант от обычного `U8`.
//!
//! Квантование требует CUDA: ядра `quantize_nvfp4`/`quantize_mxfp8` есть
//! только в GPU-бэкенде. Без карты упаковка остаётся плотной — молча
//! подсунуть непроквантованный бандл нельзя, поэтому вызывающий проверяет
//! доступность заранее ([`cuda_available`]).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use synaptix_bundle::inspect::{self, QuantKind, TensorInfo};
use synaptix_bundle::stream::{StDtype, StreamTensor, TensorStream};
use synaptix_bundle::{Error as BundleError, Result as BundleResult};
use synaptix_core::device::Device;
use synaptix_core::dtype::DType;
use synaptix_io::weights::safetensors::SafetensorsLoader;
use synaptix_io::weights::WeightLoader;

/// Имя файла-манифеста внутри бандла.
pub const MANIFEST_NAME: &str = "quant_manifest.json";

/// Возможность формата, объявляемая в `required_caps`. Читатель, который её
/// не знает, откажется открыть бандл вместо того, чтобы не найти половину
/// тензоров и списать это на битый файл. Значение — из `synaptix-bundle`,
/// чтобы писатель и проверка на чтении не разъехались.
pub const CAP_QUANT: &str = synaptix_bundle::CAP_QUANT_WEIGHTS;

pub const PACKED_SUFFIX: &str = ".qpacked";
pub const SCALES_SUFFIX: &str = ".qscales";

/// Запись манифеста про один квантованный тензор.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantEntry {
    /// `nvfp4` | `mxfp8`.
    pub format: String,
    /// Исходная форма — по ней читатель восстанавливает `n`/`k` и число
    /// срезов, не гадая по размеру блоба.
    pub shape: Vec<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuantManifest {
    pub version: u32,
    pub packed_suffix: String,
    pub scales_suffix: String,
    pub tensors: BTreeMap<String, QuantEntry>,
}

impl QuantManifest {
    fn new() -> Self {
        Self {
            version: 1,
            packed_suffix: PACKED_SUFFIX.to_string(),
            scales_suffix: SCALES_SUFFIX.to_string(),
            tensors: BTreeMap::new(),
        }
    }
}

pub fn format_key(kind: QuantKind) -> String {
    synaptix_bundle::quant_layout::format_key(kind)
}

/// Есть ли CUDA-устройство, на котором можно считать квант.
pub fn cuda_available() -> bool {
    synaptix_core::device::cuda::get(0).is_ok()
}

/// Что делать с очередным элементом плана.
enum Item {
    /// Скопировать байты источника как есть (zero-copy из mmap).
    Copy { name: String },
    /// Посчитать квант и записать упакованные веса; масштабы уйдут
    /// следующим элементом.
    Packed {
        name: String,
        kind: QuantKind,
        /// Число матриц в стопке (1 для обычного двумерного веса).
        slices: usize,
        n: usize,
        k: usize,
    },
    /// Записать масштабы, посчитанные предыдущим элементом.
    Scales { name: String },
}

/// Поток тензоров, квантующий выбранные веса на лету.
pub struct QuantizingStream {
    loader: SafetensorsLoader,
    plan: Vec<StreamTensor>,
    items: Vec<Item>,
    device: Device,
    /// Масштабы, посчитанные вместе с упакованными весами: план всегда
    /// ставит `.qscales` сразу за `.qpacked`, поэтому буфер живёт ровно один
    /// шаг и не растёт.
    pending_scales: Option<(String, Vec<u8>)>,
    manifest: QuantManifest,
}

impl QuantizingStream {
    /// Собрать поток по шардам компонента.
    ///
    /// `decide` получает имя и форму тензора и возвращает формат или `None`
    /// (оставить плотным). Решение о применимости формы принимается здесь:
    /// вернуть формат для тензора, который ядро не возьмёт, безопасно —
    /// такой тензор просто останется плотным.
    pub fn new(
        shards: &[PathBuf],
        decide: &dyn Fn(&str, &[usize]) -> Option<QuantKind>,
        device: Device,
    ) -> Result<Self, String> {
        let mut tensors: Vec<TensorInfo> = Vec::new();
        for shard in shards {
            let mut t = inspect::read_header_file(shard)
                .map_err(|e| format!("{}: {e}", shard.display()))?;
            tensors.append(&mut t);
        }
        // Порядок стабилен по имени: бандл, собранный дважды из одних
        // исходников, должен получаться одинаковым.
        tensors.sort_by(|a, b| a.name.cmp(&b.name));
        if tensors.is_empty() {
            return Err("в источнике нет тензоров".to_string());
        }

        let loader = SafetensorsLoader::open_sharded(shards).map_err(|e| e.to_string())?;

        let mut plan = Vec::with_capacity(tensors.len());
        let mut items = Vec::with_capacity(tensors.len());
        let mut manifest = QuantManifest::new();

        for t in &tensors {
            let kind = decide(&t.name, &t.shape)
                .filter(|k| inspect::quantized_bytes(&t.shape, *k).is_some())
                .filter(|_| is_float(&t.dtype));
            let Some(kind) = kind else {
                plan.push(StreamTensor {
                    name: t.name.clone(),
                    dtype: parse_dtype(&t.dtype)?,
                    shape: t.shape.clone(),
                });
                items.push(Item::Copy { name: t.name.clone() });
                continue;
            };

            let (slices, n, k) = split_shape(&t.shape)?;
            let packed_shape = packed_shape(kind, slices, n, k);
            let total = inspect::quantized_bytes(&t.shape, kind)
                .ok_or_else(|| format!("{}: форма не поддержана", t.name))?;
            let packed_bytes: usize = packed_shape.iter().product();
            let scales_bytes = total as usize - packed_bytes;

            plan.push(StreamTensor {
                name: format!("{}{PACKED_SUFFIX}", t.name),
                dtype: StDtype::U8,
                shape: packed_shape,
            });
            items.push(Item::Packed {
                name: t.name.clone(),
                kind,
                slices,
                n,
                k,
            });
            plan.push(StreamTensor {
                name: format!("{}{SCALES_SUFFIX}", t.name),
                dtype: StDtype::U8,
                shape: vec![scales_bytes],
            });
            items.push(Item::Scales { name: t.name.clone() });

            manifest.tensors.insert(
                t.name.clone(),
                QuantEntry {
                    format: format_key(kind).to_string(),
                    shape: t.shape.clone(),
                },
            );
        }

        Ok(Self {
            loader,
            plan,
            items,
            device,
            pending_scales: None,
            manifest,
        })
    }

    /// Сколько тензоров реально будет квантовано.
    pub fn quantized_count(&self) -> usize {
        self.manifest.tensors.len()
    }

    /// Манифест для записи файловым чанком. Пустой — квантовать нечего.
    pub fn manifest_json(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec_pretty(&self.manifest).map_err(|e| e.to_string())
    }

    /// Посчитать квант тензора и вернуть (упакованные веса, масштабы).
    ///
    /// Срезы стопки экспертов считаются по одному: на GPU в каждый момент
    /// живёт одна матрица, а не вся стопка на три гигабайта.
    fn quantize(
        &self,
        name: &str,
        kind: QuantKind,
        slices: usize,
        n: usize,
        k: usize,
    ) -> Result<(Vec<u8>, Vec<u8>), String> {
        // Читаем в F16 на хосте: квант-ядра принимают только F16, а держать
        // всю стопку в VRAM незачем.
        let host = self
            .loader
            .load_to(name, Device::Cpu, DType::F16)
            .map_err(|e| format!("{name}: чтение весов: {e}"))?;

        let mut packed_out: Vec<u8> = Vec::new();
        let mut scales_out: Vec<u8> = Vec::new();
        for s in 0..slices {
            let slice = if slices == 1 {
                host.clone()
            } else {
                host.narrow(0, s, 1).map_err(|e| format!("{name}: срез {s}: {e}"))?
            };
            // `narrow` отдаёт вид со смещением, а `reshape` требует
            // `offset == 0` — иначе всё, кроме нулевого эксперта, падает с
            // «non-contiguous tensor». Перенос на устройство уплотняет данные
            // сам, поэтому стопку разбираем в матрицу уже после него.
            let gpu = slice
                .to_device(self.device)
                .and_then(|t| t.contiguous())
                .and_then(|t| t.reshape((n, k)))
                .map_err(|e| format!("{name}: срез {s} на GPU: {e}"))?;
            let qw = match kind {
                QuantKind::Nvfp4 => gpu.quantize_to_nvfp4(),
                QuantKind::Mxfp8 => gpu.quantize_to_mxfp8(),
                // SQ-энкодер на GPU — этап 4 плана; ggml-энкодеров нет вовсе.
                QuantKind::Sq(_) | QuantKind::Ggml(_) => {
                    return Err(format!("{name}: формат {kind:?} упаковщик пока не пишет"));
                }
            }
            .map_err(|e| format!("{name}: квантование: {e}"))?;

            // Байты берём с хоста: `to_device(Cpu)` копирует их без
            // изменений, поэтому то, что легло в бандл, бит в бит совпадёт с
            // тем, что посчитает резидентный квант.
            let cpu = qw
                .to_device(Device::Cpu)
                .map_err(|e| format!("{name}: выгрузка кванта: {e}"))?;
            let packed = cpu
                .packed_arc()
                .ok_or_else(|| format!("{name}: упакованные веса освобождены"))?;
            packed_out.extend_from_slice(
                packed
                    .as_cpu()
                    .ok_or_else(|| format!("{name}: упакованные веса не на хосте"))?
                    .as_bytes(),
            );
            scales_out.extend_from_slice(
                cpu.scales()
                    .as_cpu()
                    .ok_or_else(|| format!("{name}: масштабы не на хосте"))?
                    .as_bytes(),
            );
        }
        Ok((packed_out, scales_out))
    }
}

impl TensorStream for QuantizingStream {
    fn plan(&self) -> &[StreamTensor] {
        &self.plan
    }

    fn write_tensor(&mut self, index: usize, w: &mut dyn Write) -> BundleResult<()> {
        let item = self
            .items
            .get(index)
            .ok_or_else(|| BundleError::Safetensors(format!("нет элемента плана {index}")))?;
        match item {
            Item::Copy { name } => {
                let (bytes, _, _) = self.loader.raw_bytes(name).ok_or_else(|| {
                    BundleError::Safetensors(format!("тензор `{name}` пропал из источника"))
                })?;
                w.write_all(bytes)?;
                Ok(())
            }
            Item::Packed { name, kind, slices, n, k } => {
                let (name, kind, slices, n, k) = (name.clone(), *kind, *slices, *n, *k);
                let (packed, scales) = self
                    .quantize(&name, kind, slices, n, k)
                    .map_err(BundleError::Safetensors)?;
                w.write_all(&packed)?;
                self.pending_scales = Some((name, scales));
                Ok(())
            }
            Item::Scales { name } => {
                let (owner, scales) = self.pending_scales.take().ok_or_else(|| {
                    BundleError::Safetensors(format!("масштабы для `{name}` не посчитаны"))
                })?;
                if owner != *name {
                    return Err(BundleError::Safetensors(format!(
                        "масштабы от `{owner}`, а ожидались от `{name}`"
                    )));
                }
                w.write_all(&scales)?;
                Ok(())
            }
        }
    }
}

/// `[N, K]` → одна матрица; `[E, N, K]` → стопка из `E`.
fn split_shape(shape: &[usize]) -> Result<(usize, usize, usize), String> {
    match shape {
        [n, k] => Ok((1, *n, *k)),
        [e, n, k] => Ok((*e, *n, *k)),
        other => Err(format!("форма {other:?} не матрица и не стопка матриц")),
    }
}

fn packed_shape(kind: QuantKind, slices: usize, n: usize, k: usize) -> Vec<usize> {
    let last = match kind {
        QuantKind::Nvfp4 => k / 2,
        QuantKind::Mxfp8 => k,
        QuantKind::Sq(_) | QuantKind::Ggml(_) => {
            synaptix_core::quant::block_row_bytes(kind.dtype(), k).unwrap_or(k)
        }
    };
    if slices == 1 {
        vec![n, last]
    } else {
        vec![slices, n, last]
    }
}

fn is_float(dtype: &str) -> bool {
    matches!(dtype, "F64" | "F32" | "F16" | "BF16")
}

fn parse_dtype(s: &str) -> Result<StDtype, String> {
    Ok(match s {
        "F64" => StDtype::F64,
        "F32" => StDtype::F32,
        "F16" => StDtype::F16,
        "BF16" => StDtype::BF16,
        "I64" => StDtype::I64,
        "I32" => StDtype::I32,
        "I16" => StDtype::I16,
        "I8" => StDtype::I8,
        "U8" => StDtype::U8,
        "BOOL" => StDtype::Bool,
        other => return Err(format!("dtype `{other}` не поддержан упаковщиком")),
    })
}

/// Решение «что чем квантовать», снятое с мастера в plain-данные: worker
/// не имеет доступа ни к сигналам, ни к контексту UI.
#[derive(Debug, Clone, Default)]
pub struct QuantDecision {
    /// Подсказка для классификатора ролей (имя компонента + purpose + id).
    pub hint: String,
    /// Выбор по ролям слоёв.
    pub by_role: Vec<(inspect::LayerRole, QuantKind)>,
    /// Точечные переопределения по группам (`inspect::group_key`).
    pub by_group: Vec<(String, QuantKind)>,
}

impl QuantDecision {
    pub fn is_empty(&self) -> bool {
        self.by_role.is_empty() && self.by_group.is_empty()
    }

    /// Формат для конкретного тензора. Групповое правило сильнее ролевого.
    pub fn for_tensor(&self, name: &str, _shape: &[usize]) -> Option<QuantKind> {
        if !self.by_group.is_empty() {
            let key = inspect::group_key(name);
            if let Some((_, kind)) = self.by_group.iter().find(|(k, _)| *k == key) {
                return Some(*kind);
            }
        }
        let role = inspect::classify_in(name, Some(&self.hint));
        self.by_role
            .iter()
            .find(|(r, _)| *r == role)
            .map(|(_, kind)| *kind)
    }
}

/// Обёртка, чтобы поток можно было отдать билдеру.
pub fn boxed(stream: QuantizingStream) -> Box<dyn TensorStream> {
    Box::new(stream)
}

/// `Arc` для манифеста — билдер принимает владение байтами.
pub fn manifest_bytes(stream: &QuantizingStream) -> Result<Arc<Vec<u8>>, String> {
    stream.manifest_json().map(Arc::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_shape_halves_the_last_axis_for_nvfp4_only() {
        assert_eq!(packed_shape(QuantKind::Nvfp4, 1, 128, 256), vec![128, 128]);
        assert_eq!(packed_shape(QuantKind::Mxfp8, 1, 128, 256), vec![128, 256]);
        // Стопка экспертов сохраняет ведущую ось.
        assert_eq!(packed_shape(QuantKind::Nvfp4, 512, 1280, 2560), vec![512, 1280, 1280]);
    }

    /// Форма упакованного тензора обязана совпадать с числом байт, которое
    /// обещает `quantized_bytes`, иначе билдер отвергнет поток: он сверяет
    /// записанное с планом.
    #[test]
    fn plan_shapes_match_the_promised_byte_counts() {
        for (shape, kind) in [
            (vec![128usize, 256], QuantKind::Nvfp4),
            (vec![17408, 5120], QuantKind::Nvfp4),
            (vec![128, 256], QuantKind::Mxfp8),
            (vec![512, 1280, 2560], QuantKind::Nvfp4),
        ] {
            let (slices, n, k) = split_shape(&shape).unwrap();
            let packed: usize = packed_shape(kind, slices, n, k).iter().product();
            let total = inspect::quantized_bytes(&shape, kind).unwrap() as usize;
            assert!(total > packed, "{shape:?}: масштабы должны занимать место");
            let scales = total - packed;
            assert_eq!(packed + scales, total);
        }
    }

    #[test]
    fn split_shape_rejects_convolutions() {
        assert!(split_shape(&[1152, 3, 2, 16, 16]).is_err());
        assert!(split_shape(&[5120]).is_err());
    }

    /// Порядок операций над срезом стопки экспертов: `narrow` даёт вид со
    /// смещением, и `reshape` по нему падает — форму можно менять только у
    /// уплотнённой копии. Ровно на этом ломалась упаковка MoE-весов начиная
    /// со второго эксперта.
    #[test]
    fn expert_slice_must_be_made_contiguous_before_reshape() {
        synaptix::init().expect("init");
        let host = synaptix_core::tensor::Tensor::from_vec(
            (0..24).map(|i| i as f32).collect::<Vec<f32>>(),
            (2usize, 3usize, 4usize),
            Device::Cpu,
        )
        .expect("тензор");

        let second = host.narrow(0, 1, 1).expect("срез");
        assert!(second.reshape((3usize, 4usize)).is_err(), "вид со смещением reshape'ить нельзя");

        let dense = second
            .to_device(Device::Cpu)
            .and_then(|t| t.contiguous())
            .and_then(|t| t.reshape((3usize, 4usize)))
            .expect("уплотнённый срез");
        assert_eq!(dense.dims(), &[3, 4]);
        // Второй эксперт начинается с 12-го элемента — данные взялись
        // от нужного среза, а не от начала стопки.
        assert_eq!(dense.to_vec2::<f32>().expect("выгрузка")[0][0], 12.0);
    }

    /// E2E квантующего потока на стопке экспертов: то, что раньше падало
    /// на втором срезе. Требует CUDA — без неё квант-ядер нет, и тест
    /// молча пропускается.
    #[test]
    fn moe_stack_streams_all_slices() {
        synaptix::init().expect("init");
        if !cuda_available() {
            eprintln!("CUDA недоступна — пропуск");
            return;
        }
        let dir = std::env::temp_dir().join("synthos-quant-moe-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("каталог");
        let path = dir.join("model.safetensors");
        let (e, n, k) = (3usize, 128usize, 256usize);
        write_test_safetensors(&path, "experts.down_proj", &[e, n, k]);

        let mut stream =
            QuantizingStream::new(&[path], &|_, _| Some(QuantKind::Mxfp8), Device::Cuda(0))
                .expect("поток");
        assert_eq!(stream.quantized_count(), 1);

        let plan: Vec<StreamTensor> = stream.plan().to_vec();
        assert_eq!(plan.len(), 2, "упакованные веса + масштабы");
        for (i, t) in plan.iter().enumerate() {
            let mut buf: Vec<u8> = Vec::new();
            stream.write_tensor(i, &mut buf).expect("запись тензора");
            let promised: usize = t.shape.iter().product();
            assert_eq!(buf.len(), promised, "{}: обещано {promised} байт", t.name);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Минимальный safetensors: 8 байт длины заголовка, JSON, затем данные.
    /// Веса — единицы в F16, чтобы масштабы получились осмысленными.
    fn write_test_safetensors(path: &std::path::Path, name: &str, shape: &[usize]) {
        let numel: usize = shape.iter().product();
        let data = vec![0x3Cu8, 0x00].repeat(numel); // 1.0 в F16, little-endian → 00 3C
        let data: Vec<u8> = data.chunks(2).flat_map(|c| [c[1], c[0]]).collect();
        let header = format!(
            r#"{{"{name}":{{"dtype":"F16","shape":{shape:?},"data_offsets":[0,{}]}}}}"#,
            data.len()
        );
        let mut out = Vec::new();
        out.extend_from_slice(&(header.len() as u64).to_le_bytes());
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(&data);
        std::fs::write(path, out).expect("запись safetensors");
    }

    #[test]
    fn manifest_round_trips() {
        let mut m = QuantManifest::new();
        m.tensors.insert(
            "model.layers.0.mlp.up_proj.weight".into(),
            QuantEntry { format: "nvfp4".into(), shape: vec![17408, 5120] },
        );
        let raw = serde_json::to_vec(&m).unwrap();
        let back: QuantManifest = serde_json::from_slice(&raw).unwrap();
        assert_eq!(back.version, 1);
        assert_eq!(back.packed_suffix, PACKED_SUFFIX);
        assert_eq!(back.tensors.len(), 1);
        assert_eq!(back.tensors["model.layers.0.mlp.up_proj.weight"].shape, vec![17408, 5120]);
    }
}
