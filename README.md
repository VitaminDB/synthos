# synthos

[![Donate via PayPal](https://img.shields.io/badge/donate-PayPal-0070ba?logo=paypal&logoColor=white)](https://paypal.me/vitamindbnfkz)

A local AI desktop studio for Rust — a node-based editor for generating video, music and
speech, with an agentic chat client, a code editor and a document knowledge base, all
running on-device on the native [synaptix](https://github.com/VitaminDB/synaptix) engine
(no Python, no torch). The UI is built with [syngui](https://github.com/VitaminDB/syngui).

## What it is

synthos turns local generative models into a visual workflow. Nodes wire models and media
together on a canvas — text or image → video with sound, lyrics → music, script → multi-voice
dialogue, audio → transcript — and run on your own GPU. Around the node editor it ships a
chat client whose agent can build and run those graphs itself, a code editor and a local
knowledge base, so the whole loop stays offline.

## Features

### Node studio
- **Video** — LTX-2.3: text-to-video, image-to-video, audio-to-video, IC-LoRA control from
  depth (Depth Anything V2) or canny edges, lip-dub and retake, two-stage sampling with a
  spatial upscaler. MiniMax-H3: text, first-frame or first+last-frame → video with
  synchronized stereo audio; a 6-step turbo preset.
- **Music** — ACE-Step: generate, cover, edit, extend, extract, repaint, retake.
- **Speech** — VoxCPM2 and OmniVoice text-to-speech with zero-shot voice cloning (the
  reference is transcribed automatically); VibeVoice for long-form multi-speaker dialogue
  from a `Speaker N:` script with up to four reference voices.
- **Transcription & diarization** — GigaAM ASR, Sortformer speaker diarization.
- **LLM** — a text-generation node inside the graph (e.g. the built-in "podcast from a
  topic" template: LLM → VibeVoice).
- **Audio** — recorder, file and FFmpeg players, mixer with dynamic inputs, 6/10/20/30-band
  equalizers, filter, gain, reverb, save-to-file; PCM streams between nodes.
- **Editor** — checkpoint nodes (ComfyUI-style) with a "keep in memory" switch per model
  family, built-in and custom templates, multi-tab graphs, run/pause/stop, per-node and
  per-run timers, a panel of loaded models, workspace autosave, Markdown annotation nodes.

### Chat
- Native synaptix inference from `.syn` bundles — NVFP4/FP8 quantization, CUDA-graph decode,
  MTP and DFlash speculative decoding, prefix-KV reuse across turns, a VRAM-aware context
  budget with automatic compaction. Models: the Qwen3 family including the Qwen3.6/3.8-27B
  hybrids, and the multimodal Muse Glimmer 30B.
- Attachments — images and video go to the vision tower, documents are inlined, audio is
  transcribed; results of a pipeline run play right in the thread.
- Agent tools — web search/read, knowledge-base retrieval, system status (VRAM/RAM, loaded
  models), **pipelines** (discover, build, edit and run node graphs from the chat, unloading
  itself for the run if VRAM is short), subagents, user-defined skills, voice input.

### Code editor
- Tree view with live git-status decorations, external-change watching, syntax
  highlighting, integrated terminals.

### Knowledge base (RAG)
- Local document collections (files, folders, PDF, URL), hybrid BM25 + vector search and an
  optional cross-encoder rerank — computed on-device, stored in bundled SQLite.

### Models
- A Hugging Face browser for fetching models, with GGUF import converted in-app to `.syn`.
- A model catalogue in Settings — one `.syn` file per model, read zero-copy via mmap.
- Syn Explorer — inspect, edit and create `.syn` bundles.

### Desktop
- A global voice-input overlay (FAB) on every page, with history.
- System appearance: follows the desktop's light/dark scheme, accent colour, title-bar
  buttons and backdrop blur.

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

An Arch Linux package is in `packaging/` (`makepkg` against the release build).

## Requirements

- Linux x86_64. Runtime: gtk3, wayland, libxkbcommon, fontconfig, a Vulkan driver, alsa, ffmpeg.
- GPU (optional, recommended): NVIDIA driver + CUDA runtime, loaded at runtime. The compile
  baseline is sm_80 (Ampere); native NVFP4 needs sm_120 (Blackwell). Without a GPU the node
  engine falls back to CPU for smoke tests and debugging.
- A Windows build is planned — the app currently depends on Linux-only components.

## Models

No model weights are shipped. Download them yourself from Hugging Face — the same files used
by ComfyUI / LM Studio — and you accept each model's licence. Some models (e.g. FLUX.1-dev,
LTX-2.3, Gemma-3) are non-commercial or otherwise restricted; check the licence before use.
Large models are packed into single `.syn` bundles with the synaptix tools; see
`docs/*_syn_bundle_2026.md`.

## Documentation

Feature notes and design write-ups live in [`docs/`](docs/) (Russian).

## Related

- [syngui](https://github.com/VitaminDB/syngui) — the GUI framework.
- [synaptix](https://github.com/VitaminDB/synaptix) — the inference and training engine.

## Support

synthos is free and open source. If it is useful to you, you can support its development with a donation via [PayPal](https://paypal.me/vitamindbnfkz).

## Licence

MIT OR Apache-2.0 — see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
