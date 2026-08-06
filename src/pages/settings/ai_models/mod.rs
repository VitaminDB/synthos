//! Settings → AI Models — конфигуратор квантования Qwen3.6 для Syn-чата.
//!
//! Состоит из:
//! - Preset dropdown: `Quality` / `Balance` / `VRAM-Saver` / `Custom`.
//! - 7 dtype dropdowns: weights/compute/kv/lm_head/embed/ssm_state/conv_state.
//! - Apply & Reload Model button: вызывает `SynModelRegistry::load(path, policy)`
//!   с текущим `last_syn_model`.
//! - Info-блок: текущая политика + примерная оценка экономии VRAM.
//!
//! Изменения любого dropdown'а переключают preset на `"custom"`. Выбор
//! пресета заполняет все dtype-поля из соответствующего helper'а в
//! `synaptix::facade::llm::QuantPolicy`.

use std::path::PathBuf;

use syngui::prelude::*;
use syngui::widgets::input::dropdown::{Dropdown, DropdownItem};
use syngui::widgets::input::toggle::Toggle;

use crate::config::SynChatQuantConfig;
use crate::context::AppCtx;
use crate::icons::*;
use crate::pages::settings::widgets::{
    compute_dtype_dropdown, kv_dtype_dropdown, row_frame, section_card, storage_dtype_dropdown,
    tied_embeddings_dropdown,
};
use crate::syn_chat::SynModelRegistry;

pub fn view() -> impl Widget {
    ScrollView::new().vertical().child(move || {
        let ctx = use_context::<AppCtx>();
        let q = ctx.syn_chat_quant.get();

        let header = Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .child(Text::new("Квантование Qwen3.6 (Syn-чат)").class("settings-section-title"))
            .child(Text::new("Политика хранения весов и кэшей модели. NVFP4 — главная экономия (~12 GB из 30 GB весов). FP8 KV-cache даёт −2 GB при длинном контексте. Изменения применяются после нажатия «Применить и перезагрузить модель».")
                .class("settings-row-desc"));

        let body = Column::new()
            .gap(24.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![
                Box::new(header) as Box<dyn Widget>,
                runtime_card(),
                preset_card(&q),
                components_card(&q),
                apply_card(),
                acestep_paths_card(),
            ]);

        Padding::all(32.0).child(
            DecoratedBox::new()
                .class("settings-page models-page ai-models-page")
                .child(body),
        )
    })
}

/// Runtime-настройки inference: per-shot контролы которые применяются
/// без перезагрузки модели (в отличие от quant-policy ниже).
fn runtime_card() -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let value = ctx.qwen36_attn_mode;
    let current = value.get_untracked();
    let items = vec![
        DropdownItem::new("fa4", "FA-4 — FlashAttention + WMMA Tensor Cores (auto)"),
        DropdownItem::new("fa2", "FA-2 — FlashAttention без Tensor Cores (скаляр)"),
        DropdownItem::new("off", "Off — reference softmax (без CUDA flash)"),
    ];
    let dropdown: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| value.set(s.to_string()))
            .class("models-active-dropdown"),
    );

    let graph_signal = ctx.qwen36_graph_decode;
    let graph_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(graph_signal.get_untracked())
            .on_change(move |v| graph_signal.set(v)),
    );

    let mtp_signal = ctx.qwen36_mtp;
    let mtp_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(mtp_signal.get_untracked())
            .on_change(move |v| mtp_signal.set(v)),
    );

    let la_signal = ctx.qwen36_la_fused;
    let la_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(la_signal.get_untracked())
            .on_change(move |v| la_signal.set(v)),
    );

    let gdr_signal = ctx.qwen36_gdr_fused;
    let gdr_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(gdr_signal.get_untracked())
            .on_change(move |v| gdr_signal.set(v)),
    );

    let mma_signal = ctx.qwen36_nvfp4_mma;
    let mma_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(mma_signal.get_untracked())
            .on_change(move |v| mma_signal.set(v)),
    );

    let gemv_signal = ctx.qwen36_nvfp4_gemv;
    let gemv_toggle: Box<dyn Widget> = Box::new(
        Toggle::with_state(gemv_signal.get_untracked())
            .on_change(move |v| gemv_signal.set(v)),
    );

    let chunk_signal = ctx.qwen36_prefill_chunk;
    let chunk_items = vec![
        DropdownItem::new("64", "64 — для 24 GB GPU + 23K context"),
        DropdownItem::new("128", "128"),
        DropdownItem::new("256", "256 — компромисс скорость/VRAM"),
        DropdownItem::new("512", "512"),
        DropdownItem::new("1024", "1024 — default (короткие промпты)"),
        DropdownItem::new("2048", "2048 — максимум throughput"),
    ];
    let chunk_current = chunk_signal.get_untracked().to_string();
    let chunk_dropdown: Box<dyn Widget> = Box::new(
        Dropdown::with_items(chunk_items)
            .selected(chunk_current)
            .on_change(move |s| {
                if let Ok(n) = s.parse::<usize>() {
                    chunk_signal.set(n);
                }
            })
            .class("models-active-dropdown"),
    );

    let sync_signal = ctx.qwen36_layer_sync;
    let sync_items = vec![
        DropdownItem::new("auto", "Auto — sync только на prefill (default)"),
        DropdownItem::new("on", "On — sync всегда (макс. экономия VRAM, −5% decode)"),
        DropdownItem::new("off", "Off — никогда (макс. скорость, риск OOM на long)"),
    ];
    let sync_current = sync_signal.get_untracked();
    let sync_dropdown: Box<dyn Widget> = Box::new(
        Dropdown::with_items(sync_items)
            .selected(sync_current)
            .on_change(move |s| sync_signal.set(s.to_string()))
            .class("models-active-dropdown"),
    );

    section_card(
        "Inference (runtime)",
        vec![
            row_frame(
                MI_SPEED,
                "Attention режим (full-attention слои)",
                "Управляет CUDA-путём в 16 full-attention слоях Qwen3.6. \
                 FA-4 — самый быстрый (Split-K decode + WMMA Tensor Cores на \
                 prefill hd=256): ×1.4 на 5K decode, ×4.3 на 23K decode, ×1.81 \
                 на 5K prefill (1262 vs 705 tok/s), ×2.53 на 23K prefill. \
                 FA-2 — тот же Split-K, но без WMMA (скалярный mma) — для \
                 bisect-отладки или если Tensor Cores ведут себя нештатно. \
                 Off — reference softmax (OOM на ≥10K токенов, baseline \
                 для регрессионных бенчей). Применяется мгновенно к следующему \
                 ответу, без перезагрузки модели.",
                dropdown,
            ),
            row_frame(
                MI_AUTO_AWESOME,
                "CUDA-graph decode (Phase D)",
                "Capture одного decode-шага в CUDA Graph + replay для каждого \
                 следующего токена. Убирает per-launch overhead на ~140 kernel \
                 calls за token. Speedup: ×1.45 на коротком контексте \
                 (17.8 → 25.8 tok/s), ×1.16 на 23K (19.4 → 22.6 tok/s) — на \
                 длинном контексте flash-attention bandwidth доминирует над \
                 launch overhead. Требует CUDA. Применяется мгновенно к \
                 следующему ответу — нужна перезагрузка модели только если \
                 устройство было создано без non-default stream (старые \
                 сборки до Phase D — теперь делается автоматически в \
                 `model_registry`).",
                graph_toggle,
            ),
            row_frame(
                MI_BOLT,
                "MTP (multi-token prediction)",
                "Спекулятивный декод на встроенной nextn-голове модели: \
                 голова предсказывает второй токен, основная модель \
                 проверяет оба за один forward. Работает только для greedy \
                 (temperature = 0) и только если в бандле есть тензоры \
                 `mtp.*` (MTP-вариант GGUF). Выдача побитово совпадает с \
                 обычным декодом. На RTX 5090 Laptop, NVFP4: 38.2 → 43.9 \
                 tok/s при приёме черновиков 80-84%. Требует перезагрузки \
                 модели — веса MTP-головы грузятся вместе с моделью.",
                mtp_toggle,
            ),
            row_frame(
                MI_BOLT,
                "Fused linear_attn prep (Phase B-1)",
                "Объединяет sigmoid + softplus + 3×repeat_interleave_cast в \
                 один CUDA kernel (5 → 1 launch на каждом из 48 linear_attn \
                 слоёв). Bit-exact с раздельным путём; default = ON. На \
                 27B-модели win в пределах шума замера (compute kernels \
                 крупнее launch overhead) — но это не вредит и удерживает \
                 меньше kernel-нод в captured CUDA graph. Отключайте только \
                 для bisect-отладки.",
                la_toggle,
            ),
            row_frame(
                MI_BOLT,
                "Fused gated_delta_rule + RmsNorm (Phase B-2)",
                "Объединяет SSM-step и RmsNormGated в один kernel: SSM-выход \
                 живёт в shared memory вместо global, RMS-фаза работает в том \
                 же block. Устраняет 8 KB read+write traffic per head per \
                 token. Требует hk == hv (true для Qwen3.6, 128 == 128). \
                 Bit-exact; default = ON. Отключайте для bisect-отладки.",
                gdr_toggle,
            ),
            row_frame(
                MI_TUNE,
                "Prefill chunk size",
                "Размер chunk'а в `prefill_chunked`. Default 1024 даёт \
                 максимальный throughput на коротких промптах, но при ≥10K \
                 tokens на 24 GB GPU пиковый KV-allocation \
                 `(B,nh,1024,T) F32 ≈ 660 MB` сверху текущего KV-ring'а \
                 приводит к OOM. Уменьшение до 64-256 разменивает -5-10% \
                 prefill-скорости на VRAM headroom. Применяется к следующему \
                 запуску `generate(...)` без reload.",
                chunk_dropdown,
            ),
            row_frame(
                MI_MEMORY,
                "Layer sync (память)",
                "`cudaStreamSynchronize` после каждого decoder-слоя. Без sync \
                 cudarc через `cudaMallocAsync` держит MLP intermediate \
                 тензоры между слоями в pool без реального reclaim'а: на \
                 prefill chunk=1024 это даёт +2-6 GB peak VRAM, что приводит \
                 к OOM на длинных промптах. Auto (default) — sync только \
                 когда T > 1 (т.е. prefill chunks): нулевая цена на decode, \
                 ~4 GB free на длинном prefill. On — sync всегда: -5% decode, \
                 -4% prefill, максимальная экономия памяти. Off — никогда: \
                 максимальная скорость, риск OOM на ≥30K промптах. \
                 Применяется к следующему `forward` без reload.",
                sync_dropdown,
            ),
            row_frame(
                MI_BOLT,
                "Native FP4 mma GEMV (Phase E.2)",
                "Native FP4 Tensor Cores для decode M=1 на проекциях NVFP4 — заменяет cuBLASLt-NVFP4 путь на shapes с N+K кратными 64. Доступно только на Blackwell consumer (sm_120a); на старых GPU silent fallback на cuBLASLt.",
                mma_toggle,
            ),
            row_frame(
                MI_BOLT,
                "Scalar NVFP4 GEMV (Phase E.2 fallback)",
                "Scalar GEMV-путь для decode M=1 на проекциях NVFP4 (без Tensor Cores). По умолчанию off — cuBLASLt-NVFP4 + Tensor Cores даёт лучшую пропускную. Включайте только для bug-bisect.",
                gemv_toggle,
            ),
        ],
    )
}

fn preset_card(q: &SynChatQuantConfig) -> Box<dyn Widget> {
    let items = vec![
        DropdownItem::new("quality", "Quality — F16 KV, F16 lm_head/embed (макс. точность)"),
        DropdownItem::new(
            "balance",
            "Balance — F16 KV + FP8 lm_head/embed (по умолчанию, 65K на 24 GB)",
        ),
        DropdownItem::new(
            "vram_saver",
            "VRAM-Saver — F16 KV + NVFP4 lm_head/embed (минимум VRAM)",
        ),
        DropdownItem::new("custom", "Custom — ручная настройка"),
    ];
    let current = q.preset.clone();
    let preset_control: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                let s = s.to_string();
                let ctx = use_context::<AppCtx>();
                if s == "custom" {
                    // Custom — только переключение имени, dtype-поля
                    // оставляем как есть (пользователь дальше крутит вручную).
                    ctx.syn_chat_quant.update(|cfg| {
                        cfg.preset = "custom".into();
                    });
                } else {
                    // Заменяем все поля из встроенного пресета.
                    let new_cfg = SynChatQuantConfig::from_preset(&s);
                    ctx.syn_chat_quant.set(new_cfg);
                }
            })
            .class("models-active-dropdown"),
    );

    section_card(
        "Пресет",
        vec![row_frame(
            MI_AUTO_AWESOME,
            "Готовая политика квантования",
            "Quality — всё в F16 (макс. точность). Balance — NVFP4 backbone + FP8 lm_head/embed: \
             65K контекста на 24 GB GPU без потери качества (≈ F16). \
             VRAM-Saver — NVFP4 lm_head/embed для минимального VRAM, но заметная просадка \
             accuracy на vocab projection. Любое ручное изменение dropdown'а ниже переключит \
             пресет на «Custom».",
            preset_control,
        )],
    )
}

fn components_card(q: &SynChatQuantConfig) -> Box<dyn Widget> {
    let mut rows: Vec<Box<dyn Widget>> = Vec::with_capacity(7);

    // 1. Weights storage
    rows.push({
        let control = storage_dtype_dropdown(q.weights_storage.clone(), |s| {
            update_quant(|cfg| cfg.weights_storage = s);
        });
        row_frame(
            MI_MEMORY,
            "Веса attn/MLP (storage)",
            "Формат хранения весов attention и MLP linear-слоёв в VRAM. NVFP4 — главная экономия (~12 GB из 30 GB), требует Blackwell sm_120. FP8 — альтернатива на Hopper/Ada/Blackwell. F16 — без квантизации.",
            control,
        )
    });

    // 2. Compute
    rows.push({
        let control = compute_dtype_dropdown(q.compute.clone(), |s| {
            update_quant(|cfg| cfg.compute = s);
        });
        row_frame(
            MI_BOLT,
            "Compute dtype (активации)",
            "Точность активаций между слоями. F16 — стандарт для GPU. BF16 — шире динамический диапазон, полезно для prefill длинного контекста. F32 — отладка.",
            control,
        )
    });

    // 3. KV-cache
    rows.push({
        let control = kv_dtype_dropdown(q.kv_dtype.clone(), |s| {
            update_quant(|cfg| cfg.kv_dtype = s);
        });
        row_frame(
            MI_DATA_OBJECT,
            "KV-cache (full-attention слои)",
            "Формат хранения K/V проекций между decode-шагами. FP8 — −50% VRAM от KV (~−2 GB при 13K tokens), один общий per-tensor scale, dequant перед matmul. F16 — без квантизации. Auto = compute dtype.",
            control,
        )
    });

    // 4. lm_head
    rows.push({
        let control = storage_dtype_dropdown(q.lm_head_storage.clone(), |s| {
            update_quant(|cfg| cfg.lm_head_storage = s);
        });
        row_frame(
            MI_ARTICLE,
            "lm_head (vocab projection, 2.5 GB в F16)",
            "Выходной слой в vocab=248K. F16 — стандартный Linear. \
             FP8 E4M3 (default balance) — native cuBLASLt FP8 GEMM, экономит ~1.2 GB, \
             accuracy ≈ F16. NVFP4 — экономия ~1.9 GB через chunked NVFP4 kernel, \
             но заметная деградация качества на vocab projection (только для \
             VRAM-Saver). Decode -10..15% против F16 из-за FP8 quantize overhead.",
            control,
        )
    });

    // 5. Embed tokens — Phase F (NVFP4 gather) / FP8 gather
    rows.push({
        let control = storage_dtype_dropdown(q.embed_storage.clone(), |s| {
            update_quant(|cfg| cfg.embed_storage = s);
        });
        row_frame(
            MI_ARTICLE,
            "embed_tokens (vocab embedding, 2.5 GB в F16)",
            "Таблица векторов токенов. F16/BF16/F32 — стандартный Embedding (gather из table). \
             FP8 E4M3 (default balance) — packed FP8 bytes + per-tensor scale + custom gather kernel, \
             экономит ~1.2 GB при vocab=248K, accuracy ≈ F16. NVFP4 (Phase F) — packed FP4 nibbles + \
             tile-major scales, экономит ~1.8 GB, но деградация качества на embed заметна — только \
             для VRAM-Saver. Q8_0/Q4_0 для embed не реализованы — fallback на F16.",
            control,
        )
    });

    // 6. Tied embeddings (Phase F)
    rows.push({
        let control = tied_embeddings_dropdown(q.tied_embeddings.clone(), |s| {
            update_quant(|cfg| cfg.tied_embeddings = s);
        });
        row_frame(
            MI_HUB,
            "Tied embeddings (lm_head ↔ embed_tokens)",
            "Auto — читает `tie_word_embeddings` из модельного config.json (для Qwen3.6 27B \
             обычно false). Force On — lm_head переиспользует веса embed_tokens (экономит весь \
             отдельный lm_head: ~2.4 GB F16 или ~0.4 GB NVFP4); на untied чекпоинте изменит \
             числовой выход. Force Off — всегда отдельный lm_head. Применяется при следующей \
             перезагрузке модели.",
            control,
        )
    });

    // 6. SSM state
    rows.push({
        let control = kv_dtype_dropdown(q.ssm_state_dtype.clone(), |s| {
            update_quant(|cfg| cfg.ssm_state_dtype = s);
        });
        row_frame(
            MI_MEMORY,
            "SSM state (Gated DeltaNet)",
            "Recurrence-state в 48 linear-attention слоях. По умолчанию F32 (HF `mamba_ssm_dtype`). BF16 экономит ~48 MB при потенциальной потере точности на длинных рекурренциях.",
            control,
        )
    });

    // 7. Conv state
    rows.push({
        let control = kv_dtype_dropdown(q.conv_state_dtype.clone(), |s| {
            update_quant(|cfg| cfg.conv_state_dtype = s);
        });
        row_frame(
            MI_TUNE,
            "Conv1d state (Gated DeltaNet)",
            "Causal conv1d left-context в linear_attn слоях. Эффект на VRAM незначителен (тысячи элементов на слой), но влияет на точность conv-update.",
            control,
        )
    });

    section_card("Компоненты", rows)
}

fn apply_card() -> Box<dyn Widget> {
    let apply_control: Box<dyn Widget> = Box::new(
        Button::new("Применить и перезагрузить модель")
            .icon(MI_AUTORENEW)
            .on_click(|| {
                let ctx = use_context::<AppCtx>();
                let policy = ctx.syn_chat_quant.get_untracked().to_policy();
                let cfg = crate::config::AppConfig::load();
                let Some(path_str) = cfg.last_syn_model.clone() else {
                    ctx.notifications.warning(
                        "Сначала откройте `.syn`-bundle в Syn-чате (правая панель → Загрузить модель).",
                    );
                    return;
                };
                let path = PathBuf::from(path_str);
                if !path.exists() {
                    ctx.notifications
                        .warning(format!("Файл не найден: {}", path.display()));
                    return;
                }
                let registry = use_context::<SynModelRegistry>();
                registry.unload();
                registry.load(path, policy);
                ctx.notifications
                    .info("Модель перезагружается с новой политикой квантования…");
            })
            .class("ai-models-apply-button"),
    );

    section_card(
        "Применение",
        vec![row_frame(
            MI_PLAY_ARROW,
            "Перезагрузить модель Syn-чата",
            "Изменения dtype применяются только при следующей загрузке модели. Нажмите, чтобы выгрузить текущую модель и поднять её заново с актуальной политикой. Если `.syn`-bundle ещё не выбран — откройте Syn-чат и нажмите «Загрузить модель».",
            apply_control,
        )],
    )
}

/// Helper для атомарного апдейта `AppCtx.syn_chat_quant` + auto-detect
/// преcета. Если новые dtype-поля совпадают с одним из встроенных пресетов
/// — `preset` устанавливается соответствующим именем, иначе `"custom"`.
fn update_quant<F: FnOnce(&mut SynChatQuantConfig)>(f: F) {
    let ctx = use_context::<AppCtx>();
    ctx.syn_chat_quant.update(|cfg| {
        f(cfg);
        // Auto-detect preset через сравнение с известными вариантами.
        let detected = cfg.to_policy().detect_preset();
        cfg.preset = detected.to_string();
    });
}

/// Card «Пути к bundle'ам ACE-Step». Два file picker'а:
/// 1. xl-bundle (`acestep_v15_xl_base.syn` / `_turbo.syn`) — DiT + projector +
///    lyric/timbre encoders + null_cond + FSQ + Detokenizer.
/// 2. vae-bundle (`acestep_vae.syn`).
///
/// Эти пути используются всеми ACE-Step нодами семейства через
/// `nodes::acestep::shared::xl_bundle_path() / vae_bundle_path()`. Они
/// глобальные, потому что внутри одного workflow обычно используется один
/// набор весов на много нод (TextEncoder + LyricEncoder + TimbreEncoder +
/// Sampler тянут общий xl-bundle). На уровне ноды остаётся только Qwen-LLM
/// path (Qwen 0.6B для Text/Lyric или Qwen 4B AR для ArLm).
fn acestep_paths_card() -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let xl_sig = ctx.acestep_xl_bundle_path;
    let vae_sig = ctx.acestep_vae_bundle_path;
    section_card(
        "ACE-Step bundles",
        vec![
            row_frame(
                MI_FOLDER_OPEN,
                "Encoder/DiT bundle (xl-base или xl-turbo)",
                "Общий `.syn` bundle с DiT 32L, TextProjector, LyricEncoder 8L, \
                 TimbreEncoder 4L, NullConditionEmb, FSQ и AudioTokenDetokenizer. \
                 Один из `acestep_v15_xl_base.syn` (full quality) или \
                 `acestep_v15_xl_turbo.syn` (8-step turbo). Используется нодами \
                 TextEncoder/LyricEncoder/TimbreEncoder/Sampler/ArLm.",
                bundle_picker(xl_sig, "Выбрать xl-base / xl-turbo bundle"),
            ),
            row_frame(
                MI_FOLDER_OPEN,
                "VAE bundle (acestep_vae.syn)",
                "ACE-Step VAE (encoder + decoder) — `acestep_vae.syn`. Используется \
                 нодами VaeEncode (audio → latent) и VaeDecode (latent → audio).",
                bundle_picker(vae_sig, "Выбрать VAE bundle"),
            ),
        ],
    )
}

/// File picker для `.syn` bundle'а с привязкой к `RwSignal<Option<String>>`.
/// Показывает имя файла справа от кнопки (или «Не выбран»).
fn bundle_picker(sig: RwSignal<Option<String>>, title: &'static str) -> Box<dyn Widget> {
    let pick_btn: Box<dyn Widget> = Box::new(
        Button::new("Выбрать…")
            .icon(MI_FOLDER_OPEN)
            .on_click(move || {
                let dlg = rfd::FileDialog::new()
                    .add_filter("Syn bundle", &["syn"])
                    .set_title(title);
                if let Some(p) = dlg.pick_file() {
                    sig.set(Some(p.to_string_lossy().to_string()));
                }
            })
            .class("ai-models-apply-button"),
    );
    let clear_btn: Box<dyn Widget> = Box::new(
        Button::new("Очистить")
            .on_click(move || sig.set(None))
            .class("ai-models-apply-button"),
    );
    let filename = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let label: String = sig
            .get()
            .map(|s| {
                std::path::Path::new(&s)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or(s)
            })
            .unwrap_or_else(|| "Не выбран".to_string());
        vec![Box::new(Text::new(label).class("settings-row-desc")) as Box<dyn Widget>]
    });
    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![pick_btn, clear_btn, Box::new(filename)]),
    )
}
