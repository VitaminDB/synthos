//! Фильтр форматов для «Скачать всё». Аналог `huggingface-cli --exclude`:
//! при включённом тумблере (см. `HuggingFaceCtx.skip_unwanted_formats`)
//! bulk-загрузка пропускает тяжёлые/несовместимые форматы (onnx / openvino /
//! fp32 / `.bin`). Паттерны держим именованным списком в одном месте, чтобы
//! не зашивать под конкретную модель.

/// Возвращает `true`, если файл нужно пропустить при «Скачать всё» (когда
/// тумблер фильтра включён). Кнопка «Скачать» на отдельном файле фильтр не
/// применяет — там качаем что угодно.
pub fn is_excluded_default(filename: &str) -> bool {
    let f = filename.to_ascii_lowercase();
    f.ends_with(".onnx")
        || f.ends_with(".onnx_data")
        || f.contains("onnx/")
        || f.starts_with("onnx")
        || f.contains("openvino")
        || f.contains("fp32")
        || f.ends_with(".bin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_compatible_formats() {
        assert!(!is_excluded_default("model.safetensors"));
        assert!(!is_excluded_default("model-00001-of-00002.safetensors"));
        assert!(!is_excluded_default("model.gguf"));
        assert!(!is_excluded_default("config.json"));
        assert!(!is_excluded_default("tokenizer.model"));
        assert!(!is_excluded_default("README.md"));
    }

    #[test]
    fn excludes_onnx() {
        assert!(is_excluded_default("model.onnx"));
        assert!(is_excluded_default("model.onnx_data"));
        assert!(is_excluded_default("onnx/model.onnx"));
        assert!(is_excluded_default("onnx_model.bin"));
    }

    #[test]
    fn excludes_openvino() {
        assert!(is_excluded_default("openvino_model.xml"));
        assert!(is_excluded_default("openvino/model.bin"));
    }

    #[test]
    fn excludes_fp32() {
        assert!(is_excluded_default("model_fp32.safetensors"));
        assert!(is_excluded_default("pytorch_model_fp32.bin"));
    }

    #[test]
    fn excludes_bin() {
        assert!(is_excluded_default("pytorch_model.bin"));
        assert!(is_excluded_default("pytorch_model-00001-of-00002.bin"));
    }

    #[test]
    fn case_insensitive() {
        assert!(is_excluded_default("Model.ONNX"));
        assert!(is_excluded_default("OpenVINO_model.xml"));
        assert!(is_excluded_default("PyTorch_Model.BIN"));
    }
}
