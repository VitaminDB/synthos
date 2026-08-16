# Muse-Glimmer-30B в `.syn`-бандле (2026-08)

meta-models/Muse-Glimmer-30B (HF, 2 safetensors-шарда, 59,6 ГБ bf16) упакована в
один `.syn` для Syn-чата и LLM-ноды. Это новая архитектура — в synaptix добавлен
крейт `synaptix-llm-muse-glimmer` и ветка фасада `muse_glimmer → MuseGlimmer`;
в synthos поменялись только подписи в UI.

## Архитектура

Плотный трансформер 29,6B + vision-энкодер 1,8B (`MuseGlimmerForConditionalGeneration`):

- 52 слоя с паттерном [sliding, sliding, sliding, full]: sliding-окно 2048 с
  RoPE θ=500k, каждый 4-й слой — полное внимание **без позиционного
  кодирования** (NoPE, `layer_rope_theta = 0`);
- GQA 32Q/2KV×128, SwiGLU FFN 19968, hidden 6656, словарь 202 048, контекст 131 072;
- gated attention: сигмоидный гейт из отдельной `self_attn.gate_proj`
  (при загрузке перемежается с `q_proj` в fused-раскладку движка `[q|gate]` по головам);
- общий scaleless QK-RMSNorm + множитель Q `qk_scale_factor=3.87`
  (свёрнут в `attn_scale = 3.87/√128`);
- sandwich-нормы Gemma2-стиля (`post_norm_eps=1e-8`), RMSNorm эмбеддинга без веса,
  финальная норма Plain (при загрузке конвертируется в OnePlus вычитанием 1);
- логиты: `20·tanh(logits·0.19611614/20)` (`output_multiplier` + softcapping);
- vision: ViT 50 слоёв (окно 448px + full каждый 4-й и последний), учёный
  pos-emb 32×32 с билинейным ресемплом (align_corners=False, zeros),
  2D-RoPE [w,h,w,h] θ=10k, pixel shuffle 2×2 (channel-major) →
  адаптер 6144→4096→4096 (gelu×2) → проекция → 6656; плейсхолдер `<|patch|>` (200092).

Chat template — канальный ATEM-протокол: `<|start|>role<|message|>…<|eot|>`,
reasoning-канал `to=self`, `Reasoning strength: low|medium|high|xhigh` в системном
блоке. Стоп-токены: `<|end_of_text|>` (200001) и `<|eot|>` (200008; фасад добавляет
его в eos-список по имени). Сэмплинг из generation_config: temp 1.0, top_p 0.95, top_k 64.

## Расположение

- Бандл: `/run/media/storage/syn_models/muse-glimmer-30b.syn`
- Исходный HF-каталог: `/run/media/storage/LLM_models/meta-models/Muse-Glimmer-30B`

## Упаковка

```sh
SRC=/run/media/storage/LLM_models/meta-models/Muse-Glimmer-30B
OUT=/run/media/storage/syn_models/muse-glimmer-30b.syn
~/Projects/2027/synaptix/target/release/syn-pack "$SRC" -o "$OUT" \
  --id "muse-glimmer-30b" --version 1.0.0 --arch muse_glimmer --purpose text-generation
```

## Проверка

```sh
# конфиг/ремапы весов из бандла
SYN_MUSE_BUNDLE=$OUT cargo test -p synaptix-llm-muse-glimmer --release probe_config_and_remaps

# паритет с transformers 5.15 (эталоны: scripts/reference/gen_muse_glimmer.py)
SYN_MUSE_BUNDLE=$OUT SYN_MUSE_REF=tests/reference_data/muse_glimmer \
  cargo test -p synaptix-llm-muse-glimmer --release --test parity -- --nocapture

# e2e-фасад (путь synthos)
SYN_MUSE_BUNDLE=$OUT cargo test -p synaptix --release --test muse_glimmer_facade_smoke -- --nocapture

# CLI
target/release/synaptix run "$OUT" "Столица Франции?" --quant nvfp4 --max-tokens 32 --temperature 0
```

## Мультимодальный CLI

```sh
# изображение (плейсхолдер <|patch|>, до 4096 merged-токенов)
target/release/synaptix run "$OUT" "Что на картинке?" --image photo.jpg --quant nvfp4 --temperature 0

# видео (fps 2, ≤96 кадров, пары кадров, ≤144 merged-токенов на группу;
# декод кадров — ffmpeg/ffprobe сабпроцессом, промпт: Time: N.Ns + <|video|>×K на группу)
target/release/synaptix run "$OUT" "Что в видео?" --video clip.mp4 --quant nvfp4 --temperature 0 --max-tokens 200
```

Vision-башня выгружается из VRAM после энкода (`release_vision`), префилл LM чанкуется
по 512 токенов, full-attention башни чанкует запросы по 1024 — иначе большая картинка
(1376 vision-токенов) не помещалась поверх резидентной NVFP4-модели на 24 ГБ.
Проверено на реальных данных: скриншоты UI описываются с чтением текста интерфейса,
видео (Том Харди с котом, Веном на пляже) — с корректным сюжетом и деталями фона.

## Скорость и контекст

Sliding-слои (39 из 52) декодят через flash-window ядро (band+causal, `window-1`
из-за полуоткрытой границы) и держат KV в ring-буфере `окно+2048` слотов с
rolling-compaction — вместо полной длины контекста. Полные NoPE-слои (13) держат
полный KV и дают ретривал по всему контексту. Итог на RTX 5090 24 ГБ (NVFP4):

| Контекст | Prefill | Decode |
|---|---|---|
| ~50 ток. (текст, повтор) | — | 37.8 tok/s (graph) / **157.7 tok/s** (lookup) |
| ~1.5k (видео-промпт) | 391 мс | 34.4 tok/s (было 4.3 до оптимизаций) |
| ~7k | 1.5 с (4700 tok/s) | 27.5 tok/s |
| ~62k | ~20 с | 13+ tok/s |

Три decode-пути (greedy → lookup, иначе → graph, фолбэк → host):
- **lookup** (prompt-lookup спекуляция, temperature=0): драфт из n-грамм уже
  известного контекста, verify-чанк до 13 токенов читает веса один раз;
  на структурном/повторном тексте ×3-5, на свободном ≈ host. Для сравнения
  Qwen3.8+MTP на том же повторе — 54.7 tok/s.
- **CUDA-graph decode**: dev-путь научен sandwich-нормам, NoPE/local-RoPE,
  embed-норме, softcap и ring-окну (`flash_splitq_*_win_dev`, Tkv из
  device-буфера; roll — между replay'ями, граф не инвалидируется).
- host: обычный цикл (CPU/несовместимый профиль).

## DFlash (блочная спекуляция)

Драфтер `meta-models/Muse-Glimmer-30B-assistant` (2.3B, 5.1 ГБ bf16) кладётся
в тот же бандл компонентом `dflash` (+ `dflash_config.json`):

```sh
syn-pack -o muse-glimmer-30b.syn --id muse-glimmer-30b --version 1.1.0   --arch muse_glimmer --purpose multimodal_llm   --component main:$SRC --component dflash:$DRAFT_DIR:dflash --files $SRC
syn-add muse-glimmer-30b.syn dflash_config.json $DRAFT_DIR/config.json
syn-meta muse-glimmer-30b.syn --component vision:model.vision_tower
```

Как работает: драфтер — 5 слоёв с **двунаправленным** sliding-вниманием, без
своей таблицы эмбеддингов и без своей головы. Его k/v — это контекст основной
модели: hidden-состояния слоёв {1, 13, 25, 37, 49} конкатенируются (5×6656) и
проецируются `encoder.fc` в 6656. Queries — «диффузионное окно» из 16 позиций:
эмбеддинг anchor-токена (последний принятый) плюс 15 mask-токенов (id 201818),
взятые из таблицы target'а **без** его RMS-нормы эмбеддинга. За один forward
драфтер денойзит всё окно, target'овский lm_head даёт 15 кандидатов, и основная
модель проверяет их одним чанком: принимаются токены до первого расхождения с
её argmax, поэтому greedy-выдача не меняется.

Кэш драфтера хранит только контекст (окно вытесняется каждый шаг) и живёт в
ring-буфере `sliding_window + 2048`; RoPE — по абсолютным позициям, поэтому
перемотка прозрачна. Внимание считает то же ядро `flash_splitq_*_win`, что и
sliding-слои target'а, но с `causal=false` (band |i−j| < window).

Переключатель — в synthos: Настройки → AI модели → «DFlash (блочная
спекуляция)»; рядом тумблер MTP для Qwen3.6/3.8. Выключение освобождает VRAM
драфтера (нужна перезагрузка модели). В CLI: `--no-dflash`.

Замеры (RTX 5090, NVFP4-модель + MXFP8-драфтер, 200 токенов свободного текста
по chat-шаблону):

| Путь | Decode |
|---|---|
| host / graph | ~38 tok/s |
| lookup (n-граммы) | 36.8 tok/s |
| **DFlash** | **58–62 tok/s** |

Черновик усечён до 8 токенов (`SYN_DFLASH_SPAN`): драфтер денойзит блок целиком,
поэтому хвост принимается редко, а verify-чанк дорожает — 61.6 tok/s против
56.8 на полных 15. Веса драфтера по умолчанию MXFP8 (`SYN_DFLASH_W`), в NVFP4
приёмка та же, но качества запаса меньше. Приёмка на свободном русском тексте
~13% черновиков (2–3 принятых токена на блок плюс бонусный), на структурном
тексте выше. Паритет с transformers: логиты кандидатов cosine 0.999987,
14 из 15 id совпадают (последний — bf16-шум на хвосте блока).

KV на 131k: ~1.75 ГБ (13 полных слоёв) + ~0.3 ГБ (ring) — полный контекст влезает
рядом с NVFP4-весами. Needle-тест: секрет из начала 62k-промпта извлекается точно
(`--prompt-file` в CLI для длинных промптов — argv ограничен 128 КБ).

## Мультимодальность Qwen3.8

Vision-тензоры (`model.visual.*`) лежали в бандле qwen3.8-27b.syn, но meta не
содержала компонент `vision`, поэтому `bundle_has_vision` возвращал false и
мультимодальность никогда не работала. Исправлено на месте (без переупаковки):
`syn-meta qwen3.8-27b.syn --component vision:model.visual`. Проверено на
скриншотах UI — Qwen3.8 читает мелкий текст интерфейса, 36 tok/s decode.

Паритет vision выверен постадийно: план окон и rope-таблицы бит-в-бит
(включая эмуляцию bf16-квантования `inv_freq`, как в `from_pretrained(dtype=bf16)`),
адаптер на эталонном входе — cosine 0.999997; расхождение выхода башни
(cosine ≈0.9967) равно собственному шуму HF между sdpa и eager (0.9972) —
bf16-хаос «массивных активаций» ViT, не ошибка порта.

## В synthos

Ничего специального: выбрать бандл в пикере Syn-чата или LLM-ноды — арх
детектируется по `config.json.model_type = muse_glimmer`, шаблон чата и
стоп-токены подхватываются фасадом из бандла. Квантование — общие пресеты
(quality/balance/vram_saver); в NVFP4 модель занимает ~17 ГБ и резидентна на 24 ГБ GPU.
DFlash-драфтер (спекулятивный декод) в этот бандл не входит — веса драфтера
опубликованы отдельным репозиторием и в v1 не поддержаны.
