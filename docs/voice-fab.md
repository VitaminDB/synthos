# Voice FAB — глобальный голосовой ввод

Описание функции: круглый FAB в правом нижнем углу на всех страницах synthos,
открывающий overlay-окно распознавания речи с анимированной аурой,
накоплением текста между паузами и страницей истории.

## Архитектура

```
AppCtx
 ├─ general
 │     ├─ voice_font_family: RwSignal<String>     // "" = sans-serif
 │     └─ voice_font_size:   RwSignal<f32>        // default 28
 ├─ audio: AudioCtx                               // запись + ASR (без изменений)
 ├─ voice: VoiceFabCtx                            // UI-состояние FAB и панели
 │     ├─ panel_open:        RwSignal<bool>
 │     ├─ awaiting_actions:  RwSignal<bool>       // true после финального Stop
 │     ├─ accumulated:       RwSignal<String>     // конкатенация всех чанков
 │     ├─ last_transcript:   RwSignal<String>
 │     └─ focus_target:      RwSignal<FocusTarget>
 └─ (истории записей больше нет — см. «История записей» ниже)
```

## Файлы

```
app/synthos/src/
  components/voice_fab/
    mod.rs           — view() — точка входа (Stack + FAB-Portal + Panel-Portal)
    fab_button.rs    — круглая FAB (Portal::BottomEnd 24/24)
    panel.rs         — overlay-окно (Portal::Center, modal, backdrop)
    aura.rs          — custom Canvas с пульсирующими кольцами и барами
    actions.rs       — реактивный actions_row + Copy/Paste/Restart/Close

app/synthos/styles/components/
  voice_fab.mss      — .fab-voice + keyframes voice-fab-idle-pulse
  voice_panel.mss    — .voice-overlay-card + keyframes voice-panel-fade-in
```

## Цикл записи

```
Idle ─click_FAB→ panel_open=true; voice_start()
                     ├─ AudioCtx.start_recording (re-use)
                     └─ vis_handle опубликован → aura живёт

─click_Pause→ voice_pause() → stop_and_send_with_sink(VoicePanel{final:false})
                              transcribing=true → on_done:
                                voice.accumulated.update(append + txt)
                              [текст появляется в окне]

─click_Mic→ voice_resume() → новый AudioRecorder
─click_Stop→ voice_stop() → ... + awaiting_actions=true

─click_Copy   → syngui::clipboard::copy(&accumulated)
─click_Paste  → paste_to_target() → chat.input ИЛИ pty (если route=code+terminal)
─click_Restart→ accumulated=""; voice_start() заново
─click_Close  → graceful stop + clear state
```

## TranscriptSink

```rust
enum TranscriptSink {
    ChatInputAppend,                            // mic-toggle на input panel
    VoicePanel { final_chunk: bool },           // глобальный FAB
}
```

Один `stop_and_send_with_sink` обрабатывает оба сценария — старый toggle
продолжает писать в `chat.input`, новый FAB — в `voice.accumulated`.

## Paste routing (без честного focus tracking)

```rust
pub fn paste_to_target(app: &AppCtx, text: &str) {
    let prefer_terminal = matches!(target, FocusTarget::Terminal)
        || (route == "code" && !matches!(target, FocusTarget::ChatInput));
    if prefer_terminal && write_to_active_terminal(text) { return; }
    paste_to_chat_input(app, text);
}
```

`write_to_active_terminal` (`pages/code_editor/state.rs`) — записывает текст
в pty stdin активного таба + `\n`. Возвращает false если терминала нет —
вызывающий делает fallback на `chat.input`.

## Аура (custom Canvas)

- Размер 280×280, `inner_r = 0.135*W`, `outer_r = 0.46*W`.
- 3 концентрических «дышащих» кольца с фазами `i/3 + t/2.5`, прозрачность
  и толщина пропорциональны RMS-уровню.
- 24 радиальных бара из `VisHandle::snapshot_bars(24)`.
- Центральная точка-микрофон (полупрозрачный диск + плотный диск 0.55*r).
- Цвет — `mss_accent` (или `mss_color`, или `#3B82F6` fallback).
- ~30 примитивов на кадр, `animated(true)`.

## Шрифт распознавания (live MSS-переменные)

`build_context` ставит `create_effect`, который при изменении `theme_key`,
`voice_font_family` или `voice_font_size` перегенерирует `theme_mss`:

```rust
create_effect(move || {
    let base = theme_data::find(&theme_key.get())
        .map(|t| t.to_mss())
        .unwrap_or_else(|| theme_data::default_theme().to_mss());
    let f = if voice_font_family.is_empty() { "sans-serif" } else { ... };
    theme_mss.set(format!("{base}\n:root {{ --voice-font-family: {f}; --voice-font-size: {s:.0}px; }}"));
});
```

В MSS:
```mss
.voice-overlay-text {
    font-family: var(--voice-font-family);
    font-size:   var(--voice-font-size);
}
```

UI настройки — Settings → Общие → «Окно голосового распознавания».

## История записей (удалена, 2026-08-27)

Страница «История голоса», её маршрут, `VoiceHistoryCtx` и модуль
`pages/voice_history` удалены целиком. Запись больше нигде не сохраняется:
`on_transcription_done` отдаёт только текст, WAV живёт ровно до конца
транскрипции (поле `PendingFinalize.wav` и параметр `wav_bytes` убраны).

Старые данные с прошлых версий остаются лежать в
`~/.config/synthos/voice/` — приложение их не читает, удалять руками.

## MVP-ограничения и future-work

1. **Pause/Resume — Вариант A** (stop+restart). Между паузами теряется
   ~50ms на init нового cpal-стрима. Future: `AudioRecorder::set_paused(bool)`
   в syngui для gapless.
2. **Focus tracking** — без honest tracking; route+terminal-эвристика для Paste.
   Future: on_focus API в TextField/Terminal или глобальный FocusTracker.
3. **Player progress** — ProgressBar `indeterminate` пока играет. Future:
   Canvas-driven прогресс по `player.position()`.
4. **Streaming ASR** — `Transcriber::transcribe_wav` блокирующий; live-режим
   потребует chunked-API в synaptix::facade::asr.

## Тесты

- `cargo test -p synthos` — все 115 тестов проходят, включая новый
  `old_general_config_without_voice_font_fields_deserializes`.
- Smoke-test: запустить synthos → FAB виден на всех страницах → click → запись →
  Pause → текст в окне → Resume → Stop → Copy/Paste/Restart.
