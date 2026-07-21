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

/* Превью кадров в VAE Decode: тёмный канвас со скруглением, мягкая тень,
 * hover-подсветка рамки (подсказка «кликни — play/pause»). */
.ltx-preview-canvas {
    width: 360px;
    height: 202px;
    background-color: #0A0C12;
    border-radius: 8px;
    border-width: 1px;
    border-color: rgba(76, 174, 235, 0.25);
    transition: border-color 180ms ease-out;
}

.ltx-preview-canvas:hover {
    border-color: rgba(76, 174, 235, 0.75);
}

/* Прогресс денойза/декода: бирюзовая полоса с плавным заполнением. */
.ltx-progress {
    width: auto;
    flex-grow: 1;
    accent-color: #4CAEEB;
    transition: width 220ms ease-out;
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
