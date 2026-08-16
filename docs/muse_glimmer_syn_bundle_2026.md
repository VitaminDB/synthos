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
