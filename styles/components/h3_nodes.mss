/* MiniMax-H3 семейство нод (8 шт): Checkpoint, TextEncoder, EmptyLatentAV,
 * Keyframe, Sampler, VaeDecode, AudioDecode, VideoSave.
 *
 * Визуальный язык — тёплая янтарно-пурпурная палитра, чтобы граф H3
 * отличался от сине-бирюзового LTX и фиолетово-розового ACE-Step.
 * Field-row'ы переиспользуют acestep-классы (общая геометрия). */

.h3-checkpoint-node,
.h3-text-encoder-node,
.h3-empty-latent-node,
.h3-keyframe-node,
.h3-sampler-node,
.h3-vae-decode-node,
.h3-audio-decode-node,
.h3-video-save-node {
    width: 380px;
}

.h3-preview-canvas {
    width: 360px;
    height: 202px;
    background: #0d0a12;
    border-radius: 8px;
    border-width: 1px;
    border-color: #3a2f4a;
    cursor: pointer;
}

.h3-preview-canvas:hover {
    border-color: #b06fd8;
}

.h3-node-info {
    font-size: 11px;
    color: #9a8fb0;
    padding: 2px 0 0 0;
}

.h3-node-running {
    font-size: 11px;
    color: #e0a458;
}

.h3-stereo-wave {
    width: 360px;
    height: 96px;
    background: #0d0a12;
    border-radius: 6px;
    border-width: 1px;
    border-color: #3a2f4a;
    accent-color: #e0a458;
}

.h3-progress {
    height: 6px;
    border-radius: 3px;
}
