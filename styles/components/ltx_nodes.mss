/* LTX-2.3 семейство нод (9 шт): Checkpoint, TextEncoder, NagPrompt,
 * SamplerStage1, Upscale, SamplerStage2, VaeDecode, AudioDecode, VideoSave.
 *
 * Визуальный язык — сине-бирюзовая палитра (граф «LTX — Text2Video»
 * отличим от фиолетово-розового ACE-Step). Field-row'ы переиспользуют
 * acestep-классы (общая геометрия label/control). */

.ltx-checkpoint-node,
.ltx-text-encoder-node,
.ltx-nag-node,
.ltx-sampler1-node,
.ltx-upscale-node,
.ltx-sampler2-node,
.ltx-vae-decode-node,
.ltx-audio-decode-node,
.ltx-video-save-node,
.ltx-image-node,
.ltx-video-input-node,
.ltx-retake-node,
.ltx-iclora-node,
.ltx-audio-input-node,
.ltx-lipdub-node,
.ltx-a2v-node {
    width: 380px;
}

/* Прогресс денойза/декода: тонкая бирюзовая полоса на тёмном треке —
 * в стилистике остальных input-виджетов ноды (см. .node-input-slider). */
.ltx-progress {
    width: auto;
    flex-grow: 1;
    height: 6px;
    accent-color: #4CAEEB;
    background-color: rgba(255, 255, 255, 0.10);
    transition: width 220ms ease-out;
}

/* Строка бара: растягивается на всю control-cell field-row'а. */
.ltx-progress-row {
    width: auto;
    flex-grow: 1;
}

/* «47% · ≈ 5:32» — процент + оценка оставшегося времени справа от бара. */
.ltx-progress-label {
    color: #98A0AD;
    font-size: 11px;
    font-weight: 500;
}

/* Кнопка отмены генерации: красная при hover, видна только при running. */
.ltx-cancel-btn {
    color: rgba(235, 110, 110, 0.85);
    transition: color 150ms ease-out, background-color 150ms ease-out;
}

.ltx-cancel-btn:hover {
    color: #FF6B6B;
    background-color: rgba(255, 107, 107, 0.12);
}

/* Pulse-анимация статус-строки активной LTX-ноды (running=true). */
.ltx-node-running {
    color: rgba(118, 196, 247, 0.95);
    animation: ltx-pulse 1100ms ease-in-out infinite alternate;
}

@keyframes ltx-pulse {
    0%   { opacity: 0.55; }
    100% { opacity: 1.00; }
}
