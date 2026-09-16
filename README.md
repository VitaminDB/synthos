# synthos

[![Donate via PayPal](https://img.shields.io/badge/donate-PayPal-0070ba?logo=paypal&logoColor=white)](https://paypal.me/vitamindbnfkz)
[![Licence: MIT OR Apache-2.0](https://img.shields.io/badge/licence-MIT%20OR%20Apache--2.0-blue)](#licence)
[![Platform: Linux](https://img.shields.io/badge/platform-Linux%20x86__64-informational)](#requirements)

A local AI desktop studio for Rust — an agentic chat client, a block-based notes workspace,
a node editor for generating video, music and speech, a code editor and a document knowledge
base. Everything runs on-device on the native [synaptix](https://github.com/VitaminDB/synaptix)
engine: no Python, no torch, no cloud. The UI is built with
[syngui](https://github.com/VitaminDB/syngui).

<!-- screenshot: main window, chat with a model answering + notes page visible -->

## What it is

synthos is one desktop app around a local GPU. A chat client runs 27B–125B models from
single-file `.syn` bundles and drives the rest of the app through tools; a notes mode holds
your documents, kanban boards, mind maps and calendar in a single project file; a node editor
wires generative models into runnable graphs — text or image → video with sound, lyrics →
music, script → multi-voice dialogue, audio → transcript. The agent can build and run those
graphs itself, write into your notes, search your knowledge base and read the web. Nothing
leaves the machine.

## Install

**Arch Linux (AUR)**

```sh
paru -S synthos-bin      # prebuilt binary from the latest release — recommended
paru -S synthos-git      # build from source (needs CUDA toolkit, ~2 h, ~15 GB disk)
```

**Other distributions** — download the tarball from
[Releases](https://github.com/VitaminDB/synthos/releases) or build from source (see
[Build](#build)). The binary is built against Arch's library versions; on other
distributions building from source is the reliable path.

## Features

### Chat and agent

- **Native inference from `.syn` bundles** — NVFP4 / MXFP8 quantization, CUDA-graph decode,
  MTP and DFlash speculative decoding, prefix-KV reuse across turns (including prompts that
  carry images), a VRAM-aware context budget with automatic compaction, and partial block
  offload that streams layers from host RAM when the model does not fit.
- **Models** — Qwen3 (dense and MoE), the Qwen3.6 / 3.8 hybrids (GatedDeltaNet + full
  attention), Qwen3.8-Flash-Next (125B MoE with a sparse-attention indexer, runs on a 24 GB
  card), Gemma-3, Gemma-4 26B A4B, Muse Glimmer 30B and Llama. Vision towers where the model
  has one — images and video go straight into the prompt.
- **Attachments** — images and video to the vision tower, documents inlined, audio
  transcribed on device; results of a pipeline run play right in the thread.
- **Agent tools** — web search and read, knowledge-base retrieval, system status (VRAM / RAM,
  loaded models), **pipelines** (discover, build, edit and run node graphs from the chat,
  unloading itself for the run if VRAM is short), **notes** (full read/write access to pages,
  blocks, boards and the calendar), `view_media` (look at a file with the model's own vision),
  a `wizard` that asks you a multiple-choice question mid-turn, subagents, user-defined
  skills, and voice input. Tool schemas are not all declared up front: with **autotools** the
  model pulls the schema it needs on demand, which keeps the prefix-KV intact.
- **The thread stays fast and honest** — the feed is virtualized (a 400-message history
  re-renders in single-digit milliseconds), generation survives switching chats, messages you
  send mid-turn queue up instead of being dropped, and every message can be edited or deleted
  in place.
- **Per-chat setup** — a library of system prompts, sampling presets (per-model defaults or
  your own), reasoning depth, a collapsible card for each side panel, and a chat can be torn
  off into a floating window.

### Notes

<!-- screenshot: notes page on the canvas — kanban board + gantt + mind map -->

- **A project is one `.syn` bundle** — pages, attachments, boards and calendars live in a
  single file you can copy or back up.
- **Block editor** — WYSIWYG over Markdown: headings, lists, checklists, toggles, tables,
  syntax-highlighted code, callouts, media with players, and charts (line, bar, pie, radar,
  gauge). Slash menu, inline toolbar, undo/redo, multi-block selection.
- **Flow or canvas** — a page is a document or a free layout: drag blocks anywhere, snap to a
  5 px grid, pin width and height, draw shapes, arrows and Bézier curves between them, drop
  images and SVG.
- **Objects** — kanban boards (labels, priority, due date, checklists, per-column timers that
  archive or escalate stale cards), Gantt charts, mind maps and a calendar with month, week,
  day and agenda views, recurring events and reminders. Any of them can be embedded into a
  page with `![[kanban:id]]`.
- **Wiring** — `[[links]]` with autocomplete, a backlink index, a force-directed graph view,
  global search across pages, and attachments stored content-addressed.
- **Life management** — tasks, a project journal, repeating items, reminders that fire while
  the app runs, and an archive. The agent drives all of it through the `notes` tool.

### Node studio

<!-- screenshot: node editor with an LTX video graph mid-run -->

- **Video** — LTX-2.3: text-to-video, image-to-video, audio-to-video, IC-LoRA control from
  depth (Depth Anything V2) or canny edges, lip-dub and retake, two-stage sampling with a
  spatial upscaler. MiniMax-H3: text, first-frame or first+last-frame → video with
  synchronized stereo audio; a 6-step turbo preset.
- **Music** — ACE-Step: generate, cover, edit, extend, extract, repaint, retake, with a
  per-stage progress readout.
- **Speech** — VoxCPM2 and OmniVoice text-to-speech with zero-shot voice cloning (the
  reference is transcribed automatically); VibeVoice for long-form multi-speaker dialogue
  from a `Speaker N:` script with up to four reference voices.
- **Transcription and diarization** — GigaAM ASR, Sortformer speaker diarization.
- **LLM** — a text-generation node inside the graph (e.g. the built-in "podcast from a topic"
  template: LLM → VibeVoice).
- **Audio** — recorder, file and FFmpeg players, mixer with dynamic inputs, 6/10/20/30-band
  equalizers, filter, gain, reverb, save-to-file; PCM streams between nodes.
- **Editor** — checkpoint nodes (ComfyUI-style) with a "keep in memory" switch per model
  family, built-in and custom templates, multi-tab graphs, run/pause/stop, per-node and
  per-run timers, a panel of loaded models, workspace autosave, Markdown annotation nodes.

### Code editor

- Tree view with live git-status decorations, external-change watching, syntax highlighting,
  integrated terminals. With no file open the editor steps aside and the terminal takes the
  centre.

### Knowledge base (RAG)

- Local document collections (files, folders, PDF, URL), hybrid BM25 + vector search and an
  optional cross-encoder rerank — computed on-device, stored in bundled SQLite.

### Models

- A Hugging Face browser for fetching models, with GGUF import converted in-app to `.syn`.
- A model catalogue in Settings — one `.syn` file per model, read zero-copy via mmap, with
  "optimal" and "custom" profiles per model.
- Syn Explorer — inspect, edit and create `.syn` bundles, with quantization applied at
  packing time.

### Desktop

- A global voice-input overlay (FAB) on every page, with history.
- System appearance: follows the desktop's light/dark scheme, accent colour, title-bar
  buttons and backdrop blur. UI scale for HiDPI.
- The interface ships in 14 languages (en, ru, de, es, fr, it, ja, kk, ko, pl, pt-BR, tr, uk,
  zh-CN).

## Performance

Measured on an RTX 5090 Laptop (24 GB) with 93 GB of system RAM — a single consumer machine,
not a server card:

| Model | Prefill | Decode |
|---|---|---|
| Gemma-4 26B A4B | 10 100 tok/s (4k prompt) | 210 tok/s |
| Qwen3.8-27B hybrid | 1 450 tok/s (3.3k prompt) | 47 tok/s (MTP + CUDA graph) |
| Qwen3.8-Flash-Next 125B MoE | 1 650 tok/s at a 260k context | 17–22 tok/s |

The 125B model holds a 262k-token context on a 24 GB card: experts live in pinned host RAM
and stream in at ~39 GB/s, the KV cache is MXFP8, and prefill runs layer-by-layer so a long
prompt never needs three copies of the activation stream. A 3k-token follow-up turn on top of
an 80k history takes 3.3 s thanks to prefix-KV reuse.

## Build

synthos is a Cargo workspace that path-depends on its two sibling repositories. Check all
three out next to each other:

```
~/Projects/
├── syngui/
├── synaptix/
└── synthos/
```

```sh
cargo build --release            # or --profile fast-release for iteration
```

The build needs the CUDA toolkit present (`nvcc` is read at build time to pin the CUDA
version; the driver itself is loaded dynamically at runtime) and system FFmpeg development
libraries. An Arch package is in [`packaging/`](packaging/) — `cargo build --release` first,
then `makepkg` against the existing binary.

## Requirements

- Linux x86_64. Runtime: gtk3, wayland, libxkbcommon, fontconfig, a Vulkan driver, alsa,
  ffmpeg.
- GPU (optional, strongly recommended): NVIDIA driver + CUDA runtime, loaded at runtime. The
  compile baseline is sm_80 (Ampere); native NVFP4 needs sm_120 (Blackwell). Without a GPU
  the node engine falls back to CPU for smoke tests and debugging.
- A Windows build is planned — the app currently depends on Linux-only components.

## Models and licences

No model weights are shipped. Download them yourself from Hugging Face — the same files used
by ComfyUI / LM Studio — and you accept each model's licence. Some models (e.g. FLUX.1-dev,
LTX-2.3, Gemma-3) are non-commercial or otherwise restricted; check the licence before use.
Large models are packed into single `.syn` bundles with the synaptix tools; see
`docs/*_syn_bundle_2026.md`.

## Documentation

Feature notes and design write-ups live in [`docs/`](docs/) (Russian) — including the
performance analyses quoted above (`qwen4exp_perf_2026.md`, `gemma4_2026.md`,
`block_offload_2026.md`).

## Related

- [syngui](https://github.com/VitaminDB/syngui) — the GUI framework.
- [synaptix](https://github.com/VitaminDB/synaptix) — the inference and training engine.

## How it is built

One developer, with Claude (Anthropic) as a daily coding assistant. The architecture, the
engine work and the benchmarks above are mine; the assistant carries a large share of the
typing, the tests and the refactors.

## Support

synthos is free and open source, written by one person. If it is useful to you, you can
support its development with a donation via [PayPal](https://paypal.me/vitamindbnfkz).

## Licence

MIT OR Apache-2.0 — see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
