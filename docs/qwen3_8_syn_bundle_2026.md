# Qwen3.8-27B в `.syn`-бандле (2026-08)

Qwen/Qwen3.8-27B (HF, 18 safetensors-шардов, 55,6 ГБ bf16) упакована в один
`.syn` для Syn-чата и LLM-ноды. Отдельной «поддержки» в движке не потребовалось:
`model_type` у модели — `qwen3_5`, то есть та же гибридная архитектура, что у
Qwen3.6-27B (`synaptix-llm-qwen3-next-hybrid` + фасад `qwen3_5 → Hybrid`).
Конфиги 3.6 и 3.8 идентичны вплоть до каждого поля (различие — только
`transformers_version`); новые веса, тот же граф.

## Архитектура (напоминание)

64 слоя: 48 × Gated DeltaNet (linear attention) + 16 × gated full attention
(каждый 4-й), dense FFN 17408, hidden 5120, головы 24Q/4KV×256,
partial rotary 0.25, словарь 248 320, контекст 262 144. MTP-голова (1 слой) —
для спекулятивного декода; vision-башня (`model.visual.*`, 27 блоков) лежит в
бандле и подхватывается `synaptix-vlm-qwen3` для мультимодального промпта.
В Syn-чате картинки идут через фасад `Llm` (`supports_media` /
`ensure_media_tower` / `encode_image` / `generate_streaming_media` диспатчат
в `HybridPipeline`, блок промпта — `<|vision_start|><|image_pad|>…<|vision_end|>`);
башню загрузчик берёт из `tensors:main` — отдельного компонента `vision` у
HF-упаковки нет. Видео: `prepare_video` в `synaptix-vlm-qwen3` (ffprobe/ffmpeg,
2 fps, ≤64 кадров, ≤4096 токенов на ролик, группа = 2 кадра → temporal-патч,
башня кодирует группы независимо), блок промпта на группу —
`<{t:.1} seconds><|vision_start|><|video_pad|>…<|vision_end|>`, как у
HF-процессора Qwen3-VL. M-RoPE движок не применяет — позиции обычные 1D,
временну́ю шкалу модель видит по текстовым таймкодам.
Гейты: z-гейт GDN — SiLU (`output_gate_type: "swish"` в конфиге), гейт
внимания — sigmoid; ровно так synaptix и считает.

## Расположение

- Бандл: `/path/to/syn_models/qwen3.8-27b.syn`
- Исходный HF-каталог: `/path/to/hf/Qwen3.8-27B`

## Упаковка

```sh
SRC=/path/to/hf/Qwen3.8-27B
OUT=/path/to/syn_models/qwen3.8-27b.syn
~/Projects/2027/synaptix/target/release/syn-pack "$SRC" -o "$OUT" \
  --id "qwen3.8-27b" --version 1.0.0 --arch qwen3.8 --purpose multimodal_llm
```

Одиночный позиционный режим syn-pack: шарды по `model.safetensors.index.json`
склеиваются в компонент `tensors:main`, все остальные файлы каталога
(config.json, tokenizer.json, chat_template.jinja, …) ложатся под своими
именами. На время упаковки нужно ~2× места (промежуточный
`*.tensors_stage0.tmp` рядом с выходом).

## Проверка

```sh
SYN=~/Projects/2027/synaptix/target/release/synaptix
$SYN run /path/to/syn_models/qwen3.8-27b.syn "Столица Франции —" \
  --quant nvfp4 --max-tokens 32 --temperature 0
```

## Что менялось в коде (2026-08-15)

Сама архитектура уже была; чинились два бага инференса, всплывшие на коротких
промптах (актуальны и для Qwen3.6):

- `synaptix-llm-common/model.rs` — `LinearAttn::forward_decode_dev` теперь
  лениво засеивает device-зеркала `conv_state_dev`/`ssm_state_dev` из
  host-состояния: короткий (s ≤ 8) префилл идёт decode-путём на свежем KV,
  где зеркал ещё нет → «conv_state_dev не инициализирован».
- `qwen3_next_hybrid/pipeline.rs` — MTP-кэш растёт на +1-2 за шаг
  (draft + advance) и не откатывается при отклонении драфта, а выделялся он
  размером с основной kv → «KV overflow» к концу генерации. Теперь `mtp_kv`
  выделяется с запасом `kv_max + max_new_tokens + 2` (кламп по RoPE-ёмкости
  MTP-модуля, новый `MtpModule::rope_capacity()`), а при исчерпании кэша
  генерация дорабатывает остаток обычным autoregressive decode.
- `synaptix-tokenizer` — регрессионный тест `tests/qwen38_template.rs` на новый
  chat_template Qwen3.8 (reasoning_effort xhigh/medium/low + raise_exception,
  preserved thinking по умолчанию, `enable_thinking=false`).
- `synthos` — только подписи UI: «Qwen3.6/3.8» в настройках AI-моделей и в
  пикере Syn-чата. Детект арх, шаблон чата и стоп-токены — из бандла.

## Chat template

Отличия от 3.6: переменная `reasoning_effort` (`xhigh` — дефолт, `medium`,
`low`) вшивает инструкцию в system-сообщение; thinking-блоки истории по
умолчанию сохраняются (`preserve_thinking` undefined → true). Рендерится
штатным движком `synaptix-tokenizer` (см. тест выше); переменные можно
передавать через `RenderOptions::with_var`.
