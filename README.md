# synthos

[![Donate via PayPal](https://img.shields.io/badge/donate-PayPal-0070ba?logo=paypal&logoColor=white)](https://paypal.me/vitamindbnfkz)

A local AI desktop studio for Rust — a node-based editor for generating video, music,
speech and images, with a chat client, a code editor and a document knowledge base, all
running on-device on the native [synaptix](https://github.com/VitaminDB/synaptix) engine
(no Python, no torch). The UI is built with [syngui](https://github.com/VitaminDB/syngui).

![synthos node editor](docs/screenshots/node-editor.png)

## What it is

synthos turns local generative models into a visual workflow. Nodes wire models and media
together on a canvas — text or image → video, lyrics → music, text → speech, audio →
transcript — and run on your own GPU. Around the node editor it ships a chat client, a
code editor and a local knowledge base, so the whole loop stays offline.

## Features

### Node studio
- **Video** — LTX-2.3: text-to-video and image-to-video, with IC-LoRA control from depth
  (Depth Anything V2) or canny edges.
- **Music** — ACE-Step: text- and lyrics-to-music.
- **Speech** — VoxCPM2 and OmniVoice text-to-speech, with voice cloning.
- **Transcription & diarization** — GigaAM / Whisper ASR, Sortformer speaker diarization.
- **Audio** — recorder, player, mixer, equalizer, filters, gain, reverb, encode/decode.
- **LLM** — Qwen3 / Qwen3-Next text-generation nodes inside a graph.

### Chat
- Native synaptix chat — device-resident decode, CUDA-graph replay, NVFP4/FP8 quantization.
- Tools — web search/read, knowledge-base retrieval, subagents, voice input.

### Code editor
- Tree view with live git-status decorations, external-change watching and syntax highlighting.

### Knowledge base (RAG)
- Local document collections (files, folders, PDF, URL), hybrid BM25 + vector search and an
  optional cross-encoder rerank — computed on-device, stored in bundled SQLite.

### Model management
- A Hugging Face browser for fetching models. Nothing is bundled.

## Screenshots

| | |
|---|---|
| ![node editor](docs/screenshots/node-editor.png) | ![chat](docs/screenshots/chat.png) |
| ![code editor](docs/screenshots/code-editor.png) | ![settings](docs/screenshots/settings.png) |

## Build

synthos is a Cargo workspace that path-depends on its two sibling repositories. Check them
out next to it:

```
~/projects/
├── syngui/
├── synaptix/
└── synthos/
```

```sh
cargo run --release              # or --profile fast-release for iteration
```

## Requirements

- Linux x86_64. Runtime: gtk3, wayland, libxkbcommon, fontconfig, a Vulkan driver, alsa, ffmpeg.
- GPU (optional, recommended): NVIDIA driver + CUDA runtime, loaded at runtime; without it
  the node engine falls back to CPU for smoke tests and debugging.
- A Windows build is planned — the app currently depends on Linux-only components.

## Models

No model weights are shipped. Download them yourself from Hugging Face — the same files used
by ComfyUI / LM Studio — and you accept each model's licence. Some models (e.g. FLUX.1-dev,
LTX-2.3, Gemma-3) are non-commercial or otherwise restricted; check the licence before use.

## Related

- [syngui](https://github.com/VitaminDB/syngui) — the GUI framework.
- [synaptix](https://github.com/VitaminDB/synaptix) — the inference and training engine.

## Support

synthos is free and open source. If it is useful to you, you can support its development with a donation via [PayPal](https://paypal.me/vitamindbnfkz).

## Licence

MIT OR Apache-2.0 — see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
