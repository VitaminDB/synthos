//! Headless smoke-раннер мультимодального пути чата (без GUI): прикрепляет
//! файл, кодирует его vision-башней загруженной модели, собирает промпт по
//! chat-шаблону и генерирует ответ.
//!
//! Проверяет ровно те стыки, которые не видно из UI-тестов:
//! число токенов-заполнителей в промпте против числа строк эмбеддингов,
//! работу `Llm::{ensure_media_tower, encode_image, encode_video}` и
//! `LlmGeneration::generate_streaming_media`.
//!
//! Запуск:
//! `vision_smoke <model.syn> <файл> ["вопрос"] [max_new_tokens] [max_image_tokens]`
//!
//! Пример:
//! `vision_smoke /run/media/storage/syn_models/muse-glimmer-30b.syn photo.png "Что на картинке?"`

use std::path::PathBuf;
use std::time::Instant;

use synaptix::facade::llm::{
    load_llm_with_policy, GenerationOptions, LlmGeneration, Message, QuantPolicy,
};
use synaptix_core::device::Device;

use synthos::syn_chat::attach::prompt::{prepare_user_message, MediaCaps};
use synthos::syn_chat::attach::{blobs, ingest};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let model_path = PathBuf::from(
        args.next()
            .ok_or("usage: vision_smoke <model.syn> <file> [prompt] [max_new] [max_img_tokens]")?,
    );
    let file = PathBuf::from(args.next().ok_or("не задан файл-вложение")?);
    let question = args.next().unwrap_or_else(|| "Что изображено на вложении?".into());
    let max_new: usize = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(64);
    let max_image_tokens: usize = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(256);

    synaptix_kernels_cpu::ensure_registered();
    synaptix_kernels_cuda::ensure_registered();

    // 1. Приём файла: CAS, метаданные, конвертация под модель.
    let t0 = Instant::now();
    let attachment = ingest::ingest(&file)?;
    println!(
        "вложение: {:?} {}×{} {} мс, модельный файл {}",
        attachment.kind,
        attachment.width,
        attachment.height,
        attachment.duration_ms,
        blobs::model_path(&attachment).display()
    );
    println!("ingest за {:?}", t0.elapsed());

    // 2. Загрузка модели.
    let t0 = Instant::now();
    let device = Device::Cuda(0);
    let (model, tokenizer) = load_llm_with_policy(&model_path, QuantPolicy::balance(), &device)
        .map_err(|e| format!("load: {e}"))?;
    println!(
        "модель загружена за {:?}; мультимодальная: {}",
        t0.elapsed(),
        model.supports_media()
    );
    if !model.supports_media() {
        return Err("у модели нет vision_config — нечего проверять".into());
    }

    // 3. Vision-башня + кодирование вложения.
    let t0 = Instant::now();
    if !model
        .ensure_media_tower()
        .map_err(|e| format!("vision tower: {e}"))?
    {
        return Err("в бандле нет тензоров vision-башни".into());
    }
    println!("vision-башня загружена за {:?}", t0.elapsed());

    let caps = MediaCaps {
        vision: true,
        max_image_tokens: (max_image_tokens > 0).then_some(max_image_tokens),
        asr: None,
    };
    let t0 = Instant::now();
    let prepared = prepare_user_message(&question, std::slice::from_ref(&attachment), &model, &caps);
    println!("кодирование вложения за {:?}", t0.elapsed());
    // Как в чате: башня отпускается сразу после кодирования, а пул
    // возвращает её память ОС — иначе prefill длинного видео-промпта
    // упирается в потолок VRAM.
    model.release_media_tower();
    let freed = synaptix::facade::llm::cuda_trim_pool(0);
    println!("vision-башня выгружена, trim: +{freed} MB");
    // Тот же дефолт, что и в приложении (`AppConfig.qwen36_prefill_chunk`).
    synaptix::facade::llm::set_prefill_chunk_size(256);

    let media_tokens: usize = prepared.media.iter().map(|m| m.tokens).sum();
    if prepared.media.is_empty() {
        return Err("вложение не попало в медиа-поток (см. лог выше)".into());
    }
    println!("медиа: {} шт., {media_tokens} vision-токенов", prepared.media.len());

    // 4. Промпт по chat-шаблону модели.
    let messages = vec![Message::user(prepared.text.clone())];
    let prompt = tokenizer
        .apply_chat_template_ex_tools(&messages, true, false, None)
        .map_err(|e| format!("template: {e}"))?;
    let prompt_ids = tokenizer.encode(&prompt).map_err(|e| format!("encode: {e}"))?;
    println!("промпт: {} символов / {} токенов", prompt.len(), prompt_ids.len());

    // Главная инварианта: заполнителей в промпте ровно столько же, сколько
    // строк эмбеддингов отдала башня. Расхождение — гарантированная ошибка
    // на prefill, и ловить её лучше здесь.
    let pad_ids = media_pad_ids(&tokenizer);
    let pads_in_prompt = prompt_ids.iter().filter(|id| pad_ids.contains(id)).count();
    println!("токенов-заполнителей в промпте: {pads_in_prompt}");
    if pads_in_prompt != media_tokens {
        return Err(format!(
            "рассинхрон: заполнителей {pads_in_prompt}, эмбеддингов {media_tokens}"
        ));
    }

    // 5. Генерация по медиа-промпту.
    let opts = GenerationOptions {
        max_new_tokens: max_new,
        max_seq_len: prompt_ids.len() + max_new + 128,
        temperature: 0.0,
        top_k: 0,
        top_p: 1.0,
        min_p: 0.0,
        seed: 0,
        repeat_penalty: 1.0,
        repeat_last_n: 0,
        presence_penalty: 0.0,
        frequency_penalty: 0.0,
    };
    let mut runner = LlmGeneration::new(&model, opts);
    runner.set_stop_tokens(tokenizer.eos_ids().to_vec());

    let media_refs: Vec<_> = prepared.media.iter().collect();
    let mut out = String::new();
    let t0 = Instant::now();
    runner
        .generate_streaming_media(&prompt_ids, &tokenizer, &media_refs, |_id, delta| {
            print!("{delta}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            out.push_str(delta);
            true
        })
        .map_err(|e| format!("generate: {e}"))?;
    println!();
    println!("сгенерировано {} символов за {:?}", out.len(), t0.elapsed());

    if out.trim().is_empty() {
        return Err("модель не выдала ни одного токена".into());
    }
    println!("OK");
    Ok(())
}

/// id токенов-заполнителей медиа: `<|patch|>` для картинок, `<|video|>`
/// для видео. Берём через сам токенайзер, чтобы не хардкодить числа.
fn media_pad_ids(tokenizer: &synaptix::facade::llm::LlmTokenizer) -> Vec<u32> {
    ["<|patch|>", "<|video|>"]
        .iter()
        .filter_map(|t| match tokenizer.encode(t) {
            Ok(ids) if ids.len() == 1 => Some(ids[0]),
            _ => None,
        })
        .collect()
}
