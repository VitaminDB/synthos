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
//!
//! `VSMOKE_ROUNDS=N` повторяет круг «башня → кодирование → генерация» N раз
//! на одной загруженной модели. Первый круг проходит на свежей VRAM и потому
//! проходит всегда; ломается обычно второй — башню приходится поднимать
//! поверх памяти, которую оставил после себя KV-ring прошлой генерации.
//! `VSMOKE_KEEP_CACHE=1` оставляет кэш эмбеддингов живым между кругами
//! (тогда второй круг — это регенерация ответа на то же вложение).
//!
//! `VSMOKE_PROBE_MB=N` после генерации пробует занять N МБ одним куском.
//! Разделяет два похожих симптома: «память утекла» и «память лежит
//! свободными блоками в пуле, но не видна драйверу». Печатаемый рядом
//! разбор `[VRAM …]` показывает reserved/used пула и число живых
//! аллокаций — если живых столько же, сколько до генерации, значит не
//! утекло ничего.
//!
//! Приблизиться к ходу чата (23.09.2026, «модель не видит картинку» в MyLife):
//! несколько файлов через запятую — одно сообщение с N вложениями;
//! `SMOKE_SESSION=1` — путь с префикс-KV (`generate_streaming_cached_media`);
//! `SMOKE_SYSTEM=1` — системный промпт чата без инструментов;
//! `SMOKE_SHARE_PATH=1` — галочка «путь к файлу»; `SMOKE_PRINT_PROMPT=1` —
//! промпт с ужатыми заполнителями; сэмплинг — `SMOKE_TEMP`, `SMOKE_TOP_P`,
//! `SMOKE_PRESENCE` (сид = номер круга).

use std::path::PathBuf;
use std::time::Instant;

use synaptix::facade::llm::{
    load_llm_with_policy, GenerationOptions, LlmGeneration, Message,
};
use synaptix_core::device::Device;

use synthos::agent::state::MsgAttachment;
use synthos::syn_chat::attach::prompt::{self as prompt, prepare_user_message, MediaCaps};
use synthos::syn_chat::channel_parser::{ChannelIds, ChannelParser};
use synthos::syn_chat::attach::{blobs, ingest, media_cache};
use synthos::syn_chat::model_registry::{reclaim_vram, vram_free_mb};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let model_path = PathBuf::from(
        args.next()
            .ok_or("usage: vision_smoke <model.syn> <file> [prompt] [max_new] [max_img_tokens]")?,
    );
    // Несколько вложений — через `,` (как одно сообщение с N файлами).
    let files: Vec<PathBuf> = args
        .next()
        .ok_or("не задан файл-вложение")?
        .split(',')
        .map(PathBuf::from)
        .collect();
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
    // `SMOKE_SHARE_PATH=1` — галочка «передать модели путь к файлу».
    let share_path = std::env::var("SMOKE_SHARE_PATH").is_ok_and(|v| v != "0");
    let mut attachments = Vec::new();
    for file in &files {
        let mut attachment = ingest::ingest(file)?;
        attachment.share_path = share_path;
        println!(
            "вложение: {:?} {}×{} {} мс, модельный файл {}",
            attachment.kind,
            attachment.width,
            attachment.height,
            attachment.duration_ms,
            blobs::model_path(&attachment).display()
        );
        attachments.push(attachment);
    }
    println!("ingest за {:?}", t0.elapsed());

    // 2. Загрузка модели.
    let t0 = Instant::now();
    let device = Device::Cuda(0);
    // Тот же режим, что у чата по умолчанию: «оптимальный» профиль бандла, а
    // не общий `balance` — иначе смоук гоняет не тот путь, что приложение.
    let policy = synaptix::facade::llm::optimal_profile(&model_path).policy;
    let (model, tokenizer) =
        load_llm_with_policy(&model_path, policy, &device).map_err(|e| format!("load: {e}"))?;
    println!(
        "модель загружена за {:?}; мультимодальная: {}",
        t0.elapsed(),
        model.supports_media()
    );
    if !model.supports_media() {
        return Err("у модели нет vision_config — нечего проверять".into());
    }

    let caps = MediaCaps {
        vision: true,
        vision_error: None,
        max_image_tokens: (max_image_tokens > 0).then_some(max_image_tokens),
        asr: None,
        doc_budget: None,
    };
    // Тот же дефолт, что и в приложении (`AppConfig.qwen36_prefill_chunk`).
    synaptix::facade::llm::set_prefill_chunk_size(256);

    let rounds: usize = std::env::var("VSMOKE_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let keep_cache = std::env::var("VSMOKE_KEEP_CACHE").is_ok();
    for round in 1..=rounds {
        if round > 1 {
            println!("\n===== круг {round}/{rounds} =====");
            if !keep_cache {
                media_cache::clear();
            }
        }
        // SAFETY: однопоточный смоук, переменная читается только ниже.
        unsafe { std::env::set_var("SMOKE_SEED", round.to_string()) };
        round_once(&model, &tokenizer, &attachments, &question, &caps, max_new)?;
    }
    println!("OK");
    Ok(())
}

/// Один круг чата: поднять башню → закодировать вложение → отпустить башню →
/// собрать промпт → сгенерировать ответ. Ровно то, что делает `syn_chat` на
/// каждой отправке и регенерации.
fn round_once(
    model: &synaptix::facade::llm::Llm,
    tokenizer: &synaptix::facade::llm::LlmTokenizer,
    items: &[MsgAttachment],
    question: &str,
    caps: &MediaCaps,
    max_new: usize,
) -> Result<(), String> {
    vram_report("до башни");
    // 3. Vision-башня + кодирование вложения — тем же путём, что и чат:
    // башня поднимается только под вложения, которых ещё нет в кэше
    // эмбеддингов, а её провал деградирует вложение в текстовую строку.
    let needs = prompt::needs_tower(items, caps);
    println!("башня нужна: {needs}");
    let t0 = Instant::now();
    let tower = prompt::ensure_tower(model, needs);
    let mut caps = caps.clone();
    caps.vision = tower.is_ok();
    match &tower {
        Ok(()) => println!("vision-башня загружена за {:?}", t0.elapsed()),
        Err(reason) if reason.is_empty() => {}
        Err(reason) => {
            println!("vision-башня недоступна: {reason}");
            caps.vision_error = Some(reason.clone());
        }
    }

    let t0 = Instant::now();
    let prepared = prepare_user_message(question, items, model, &caps);
    println!("кодирование вложения за {:?}", t0.elapsed());
    // Как в чате: башня отпускается сразу после кодирования, а пул
    // возвращает её память ОС — иначе prefill длинного видео-промпта
    // упирается в потолок VRAM.
    model.release_media_tower();
    let (freed, descs) = reclaim_vram(Device::Cuda(0));
    println!(
        "vision-башня выгружена, trim: +{freed} MB ({descs} TMA-деск.); VRAM свободно {} MB",
        vram_free_mb()
    );

    let media_tokens: usize = prepared.media.iter().map(|m| m.tokens).sum();
    if prepared.media.is_empty() {
        return Err("вложение не попало в медиа-поток (см. лог выше)".into());
    }
    println!("медиа: {} шт., {media_tokens} vision-токенов", prepared.media.len());

    // 4. Промпт по chat-шаблону модели.
    // `SMOKE_SYSTEM=1` — системный промпт чата без инструментов, как в
    // приложении (`system_prompt::build`).
    let mut messages = Vec::new();
    if std::env::var("SMOKE_SYSTEM").is_ok_and(|v| v != "0") {
        let env = synthos::syn_chat::system_prompt::PromptEnv {
            date: "2026-09-23".into(),
            os: "linux (x86_64)".into(),
            cwd: "/home/master".into(),
            tools: Vec::new(),
            pool: Vec::new(),
            max_turns: 30,
            user_prompt: String::new(),
            language: "Русский".into(),
        };
        messages.push(Message::system(synthos::syn_chat::system_prompt::build(&env)));
    }
    messages.push(Message::user(prepared.text.clone()));
    let prompt = tokenizer
        .apply_chat_template_ex_tools(&messages, true, false, None)
        .map_err(|e| format!("template: {e}"))?;
    let prompt_ids = tokenizer.encode(&prompt).map_err(|e| format!("encode: {e}"))?;
    println!("промпт: {} символов / {} токенов", prompt.len(), prompt_ids.len());
    if std::env::var("SMOKE_PRINT_PROMPT").is_ok() {
        let mut shown = prompt.clone();
        for pad in ["<|image_pad|>", "<|video_pad|>", "<|patch|>"] {
            while shown.contains(&format!("{pad}{pad}")) {
                shown = shown.replace(&format!("{pad}{pad}"), pad);
            }
        }
        println!("----- промпт -----\n{shown}\n------------------");
    }
    let pad_ids = media_pad_ids(tokenizer);

    // Главная инварианта: заполнителей в промпте ровно столько же, сколько
    // строк эмбеддингов отдала башня. Расхождение — гарантированная ошибка
    // на prefill, и ловить её лучше здесь.
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
        temperature: std::env::var("SMOKE_TEMP").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0),
        top_k: 20,
        top_p: std::env::var("SMOKE_TOP_P").ok().and_then(|v| v.parse().ok()).unwrap_or(0.95),
        min_p: 0.0,
        seed: std::env::var("SMOKE_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0),
        repeat_penalty: 1.0,
        repeat_last_n: 64,
        presence_penalty: std::env::var("SMOKE_PRESENCE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0),
        frequency_penalty: 0.0,
    };
    let mut runner = LlmGeneration::new(model, opts);
    runner.set_stop_tokens(tokenizer.eos_ids().to_vec());
    vram_report("под KV-ring");

    let media_refs: Vec<_> = prepared.media.iter().collect();
    // Ровно тот же разбор, что и в чате: у канальных моделей заголовки
    // сообщений (` to=self`) не должны попадать в ответ.
    let mut channel = ChannelIds::detect(tokenizer).map(ChannelParser::new);
    println!(
        "протокол хода: {}",
        if channel.is_some() { "канальный (to=self/to=user)" } else { "ChatML" }
    );
    let mut out = String::new();
    let mut body = String::new();
    let mut thinking = String::new();
    let t0 = Instant::now();
    // `SMOKE_SESSION=1` — путь чата с префикс-KV: сессия и
    // `generate_streaming_cached_media`, как в `syn_chat::session`.
    let mut session = if std::env::var("SMOKE_SESSION").is_ok_and(|v| v != "0") {
        let ctx = std::env::var("SMOKE_SESSION_CTX").ok().and_then(|v| v.parse().ok()).unwrap_or(40_000);
        let s = model.new_kv_session(ctx, max_new).map_err(|e| format!("session: {e}"))?;
        println!("сессия префикс-KV: {}", s.is_some());
        s
    } else {
        None
    };
    let on_token = |id, delta: &str| {
            print!("{delta}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            out.push_str(delta);
            match channel.as_mut() {
                Some(p) => {
                    let split = p.feed(id, delta);
                    body.push_str(&split.body);
                    thinking.push_str(&split.thinking);
                }
                None => body.push_str(delta),
            }
            true
        };
    match session.as_mut() {
        Some(sess) => runner
            .generate_streaming_cached_media(sess, &prompt_ids, tokenizer, &media_refs, on_token)
            .map(|_| ()),
        None => runner.generate_streaming_media(&prompt_ids, tokenizer, &media_refs, on_token),
    }
    .map_err(|e| format!("generate: {e}"))?;
    println!();
    println!("сгенерировано {} символов за {:?}", out.len(), t0.elapsed());
    // KV-ring живёт внутри пайплайна и умирает вместе с вызовом generate —
    // но его блоки остаются в пуле аллокатора. Возврат ОС здесь показывает,
    // с какой свободной VRAM стартует следующая отправка.
    drop(runner);
    let (freed, descs) = reclaim_vram(Device::Cuda(0));
    println!("после генерации trim: +{freed} MB ({descs} TMA-деск.)");
    vram_report("после генерации");
    // Проба: резерв пула сверх `used` не виден в cuMemGetInfo, но пулу
    // доступен. Если аллокация больше «свободного» проходит — память не
    // потеряна, врёт термометр, по которому принимаются решения.
    if let Ok(mb) = std::env::var("VSMOKE_PROBE_MB") {
        let mb: usize = mb.parse().unwrap_or(0);
        let t0 = Instant::now();
        let probe = synaptix_core::tensor::Tensor::zeros(
            vec![mb * 1024 * 1024],
            synaptix_core::dtype::DType::U8,
            Device::Cuda(0),
        );
        match probe {
            Ok(t) => {
                println!(
                    "проба {mb} MB: успех за {:?}, свободно после {} MB",
                    t0.elapsed(),
                    vram_free_mb()
                );
                drop(t);
            }
            Err(e) => println!("проба {mb} MB: провал — {e}"),
        }
        let (freed, _) = reclaim_vram(Device::Cuda(0));
        println!("после пробы trim: +{freed} MB, свободно {} MB", vram_free_mb());
    }

    if out.trim().is_empty() {
        return Err("модель не выдала ни одного токена".into());
    }
    if channel.is_some() {
        println!("--- размышления ({} симв.) ---\n{}", thinking.len(), thinking.trim());
        println!("--- ответ ({} симв.) ---\n{}", body.len(), body.trim());
        if body.contains("to=user") || body.contains("to=self") {
            return Err("заголовок канала утёк в текст ответа".into());
        }
    }
    Ok(())
}

/// id токенов-заполнителей медиа: `<|patch|>` для картинок, `<|video|>`
/// для видео. Берём через сам токенайзер, чтобы не хардкодить числа.
/// Разбор свободной VRAM: сколько держит пул, сколько живо по нашему учёту
/// и какие классы аллокаций сидят наверху. Разница между «свободно» и
/// «живо» — то, что утекло мимо пула (JIT-модули, cuBLAS-воркспейсы,
/// граф-буферы): их не вернёт ни trim, ни сброс кэшей ядер.
fn vram_report(tag: &str) {
    let mb = |(r, u): (u64, u64)| (r / (1024 * 1024), u / (1024 * 1024));
    let (dres, dused) = synaptix_core::memory::cuda_pool::cuda_mempool_stats(0)
        .map(mb)
        .unwrap_or((0, 0));
    let (wres, wused) = synaptix_core::device::cuda::weights_pool_stats(0)
        .map(mb)
        .unwrap_or((0, 0));
    let all = synaptix_core::memory::cuda_pool::live_alloc_top(100_000);
    let total: isize = all.iter().map(|(_, c)| *c).sum();
    let small: isize = all.iter().filter(|(b, _)| *b < 65_536).map(|(_, c)| *c).sum();
    let top: Vec<String> = all
        .iter()
        .take(4)
        .map(|(bytes, count)| format!("{count}x{}KB", bytes / 1024))
        .collect();
    println!(
        "[VRAM {tag}] свободно {} MB; default-пул {dres}/{dused} MB, weights-пул \
         {wres}/{wused} MB (reserved/used); живых {:.0} MB в {total} аллокациях \
         (мелких <64KB: {small}); топ: {}",
        vram_free_mb(),
        synaptix_core::memory::cuda_pool::cuda_allocated_mb(),
        top.join(", ")
    );
}

fn media_pad_ids(tokenizer: &synaptix::facade::llm::LlmTokenizer) -> Vec<u32> {
    // Muse Glimmer и Qwen-гибрид: у каждой семьи свои токены-заполнители.
    ["<|patch|>", "<|video|>", "<|image_pad|>", "<|video_pad|>"]
        .iter()
        .filter_map(|t| match tokenizer.encode(t) {
            Ok(ids) if ids.len() == 1 => Some(ids[0]),
            _ => None,
        })
        .collect()
}
