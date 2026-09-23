//! Воспроизведение аудио-вложений: декод, play/pause, тик позиции.
//!
//! Общая механика для полноэкранного просмотрщика ([`super::media_viewer`])
//! и инлайн-карточки в ленте ([`super::media_inline`]): декодируем файл в
//! фоне (ffmpeg — секунды), играем через `AudioPlayer`, позицию тикаем в
//! сигнал отдельным потоком.
//!
//! Состояние инлайн-карточки живёт в реестре по sha вложения
//! ([`inline_state`]), а не в самой карточке: лента пересобирается на каждое
//! сообщение (и на каждый токен стрима), и свой плеер у каждой сборки
//! осиротил бы играющий трек — кнопка новой карточки его уже не видела и
//! запускала второй.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use syngui::audio::{AudioBuffer, AudioPlayer};
use syngui::prelude::*;

use crate::syn_chat::attach::blobs;
use crate::syn_chat::state::MsgAttachment;

/// Состояние одного аудио-проигрывателя. `key` — sha вложения: по нему
/// понимаем, что показывают уже другой файл и декодировать надо заново.
#[derive(Clone, Copy)]
pub struct AudioSignals {
    pub key: RwSignal<String>,
    pub buf: RwSignal<Option<Arc<AudioBuffer>>>,
    pub pos: RwSignal<f32>,
    pub playing: RwSignal<bool>,
}

impl Default for AudioSignals {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioSignals {
    pub fn new() -> Self {
        Self {
            key: use_signal(String::new()),
            buf: use_signal(None),
            pos: use_signal(0.0),
            playing: use_signal(false),
        }
    }

    /// Доля проигранного [0..1] по текущему буферу.
    pub fn progress(&self) -> f32 {
        let total = self.duration();
        if total > 0.0 {
            (self.pos.get() / total).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Длительность декодированного буфера в секундах (0.0, пока не готов).
    pub fn duration(&self) -> f32 {
        let Some(buf) = self.buf.get() else {
            return 0.0;
        };
        buf.pcm.len() as f32 / buf.sample_rate.max(1) as f32 / buf.channels.max(1) as f32
    }
}

pub type PlayerSlot = Arc<Mutex<Option<AudioPlayer>>>;

pub fn new_slot() -> PlayerSlot {
    Arc::new(Mutex::new(None))
}

/// Декодировать вложение в фоне, если это ещё не сделано.
pub fn ensure_decoded(a: &MsgAttachment, signals: AudioSignals, player: &PlayerSlot) {
    if signals.key.get_untracked() == a.sha256 {
        return;
    }
    stop(player);
    signals.key.set_always(a.sha256.clone());
    signals.buf.set_always(None);
    signals.pos.set_always(0.0);
    signals.playing.set_always(false);

    let path = blobs::source_path(a);
    let sha = a.sha256.clone();
    std::thread::spawn(move || {
        let decoded = crate::pages::node_editor::nodes::decode::decode_file(&path);
        syngui::async_runtime::run_on_main_thread(move || {
            // Пока декодировали, пользователь мог перелистнуть дальше.
            if signals.key.get_untracked() != sha {
                return;
            }
            match decoded {
                Ok(buf) => signals.buf.set(Some(Arc::new(buf))),
                Err(e) => log::warn!("[media] декод аудио: {e}"),
            }
        });
    });
}

pub fn toggle(player: &PlayerSlot, signals: AudioSignals) {
    let Ok(mut guard) = player.lock() else {
        return;
    };
    match guard.as_ref() {
        Some(p) if p.is_paused() => {
            p.resume();
            signals.playing.set(true);
            spawn_position_poller(player.clone(), signals);
        }
        Some(p) => {
            p.pause();
            signals.playing.set(false);
        }
        None => {
            let Some(buf) = signals.buf.get_untracked() else {
                return;
            };
            // AudioPlayer принимает моно-поток; для стерео-исходника
            // усредняем каналы — превью, точность здесь не нужна.
            let pcm: Arc<[f32]> = if buf.channels > 1 {
                let ch = buf.channels as usize;
                buf.pcm
                    .chunks(ch)
                    .map(|c| c.iter().sum::<f32>() / ch as f32)
                    .collect()
            } else {
                buf.pcm.clone()
            };
            match AudioPlayer::start(pcm, buf.sample_rate) {
                Ok(p) => {
                    // Перемотка до первого play двигает только `pos` (плеера
                    // ещё нет) — переносим её в свежий плеер, иначе он
                    // заиграл бы с нуля, а курсор стоял бы на месте клика.
                    let from = signals.pos.get_untracked();
                    if from > 0.0 {
                        let _ = p.seek_seconds(from as f64);
                    }
                    *guard = Some(p);
                    signals.playing.set(true);
                    spawn_position_poller(player.clone(), signals);
                }
                Err(e) => log::warn!("[media] воспроизведение: {e}"),
            }
        }
    }
}

/// Перемотать на долю `t` ∈ [0..1] от длительности. Работает и до первого
/// play: тогда двигается только курсор, а `toggle` стартует плеер с этой
/// позиции.
pub fn seek(player: &PlayerSlot, signals: AudioSignals, t: f32) {
    let total = signals.duration();
    if total <= 0.0 {
        return;
    }
    let secs = (t.clamp(0.0, 1.0) * total) as f64;
    if let Ok(guard) = player.lock() {
        if let Some(p) = guard.as_ref() {
            if let Err(e) = p.seek_seconds(secs) {
                log::warn!("[media] перемотка: {e}");
            }
        }
    }
    signals.pos.set(secs as f32);
}

/// Тикает позицию воспроизведения в сигнал, пока звук играет.
fn spawn_position_poller(player: PlayerSlot, signals: AudioSignals) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(80));
        let snapshot = {
            let Ok(guard) = player.lock() else {
                return;
            };
            guard
                .as_ref()
                .map(|p| (p.position_seconds() as f32, p.is_paused(), p.is_done()))
        };
        let Some((pos, paused, done)) = snapshot else {
            return;
        };
        syngui::async_runtime::run_on_main_thread(move || {
            signals.pos.set(pos);
            if done {
                signals.playing.set(false);
            }
        });
        if paused || done {
            return;
        }
    });
}

pub fn stop(player: &PlayerSlot) {
    if let Ok(mut guard) = player.lock() {
        if let Some(p) = guard.take() {
            p.stop();
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Реестр инлайн-карточек ленты
// ─────────────────────────────────────────────────────────────────────────────

/// Аренда записи реестра: её держат замыкания карточки. Пока на экране есть
/// хоть одна карточка файла, запись и плеер живы; с последней отпущенной
/// арендой трек останавливается.
pub struct CardLease(());

impl Drop for CardLease {
    fn drop(&mut self) {
        // Уборка отложенная: при пересборке ленты старая карточка может
        // умереть раньше, чем новая возьмёт запись, — к моменту уборки новая
        // аренда уже на месте.
        syngui::async_runtime::run_on_main_thread(sweep_inline);
    }
}

struct InlineEntry {
    signals: AudioSignals,
    player: PlayerSlot,
    lease: Weak<CardLease>,
}

thread_local! {
    static INLINE: RefCell<HashMap<String, InlineEntry>> = RefCell::new(HashMap::new());
}

/// Остановить и забыть все инлайн-карточки: смена чата, закрытие ленты.
/// В отличие от [`sweep_inline`] снимает и играющие.
pub fn stop_inline_all() {
    let all: Vec<InlineEntry> = INLINE.with(|reg| reg.borrow_mut().drain().map(|(_, e)| e).collect());
    for e in all {
        stop(&e.player);
        e.signals.key.set_always(String::new());
        e.signals.buf.set_always(None);
        e.signals.playing.set_always(false);
        e.signals.pos.set_always(0.0);
    }
}

/// Состояние инлайн-карточки файла `sha`: общее для всех сборок ленты (и
/// для нескольких карточек одного файла). Аренду карточка обязана держать,
/// пока показана.
pub fn inline_state(sha: &str) -> (AudioSignals, PlayerSlot, Arc<CardLease>) {
    INLINE.with(|reg| {
        let mut reg = reg.borrow_mut();
        let entry = reg.entry(sha.to_string()).or_insert_with(|| InlineEntry {
            signals: AudioSignals::new(),
            player: new_slot(),
            lease: Weak::new(),
        });
        let lease = entry.lease.upgrade().unwrap_or_else(|| {
            let lease = Arc::new(CardLease(()));
            entry.lease = Arc::downgrade(&lease);
            lease
        });
        (entry.signals, entry.player.clone(), lease)
    })
}

/// Убрать записи, чьих карточек больше нет: удаление сообщения, очистка
/// ленты, уход строки из окна виртуального списка.
///
/// Играющий трек уборка не трогает: в виртуальной ленте строка уходит из
/// дерева, стоит прокрутить её за край окна, и музыка обрывалась бы на
/// ровном месте. Такую запись подхватит обратно та же карточка, когда
/// строка вернётся в окно. Полная остановка — [`stop_inline_all`].
pub fn sweep_inline() {
    let dead: Vec<InlineEntry> = INLINE.with(|reg| {
        let mut reg = reg.borrow_mut();
        let keys: Vec<String> = reg
            .iter()
            .filter(|(_, e)| e.lease.strong_count() == 0 && !e.signals.playing.get_untracked())
            .map(|(k, _)| k.clone())
            .collect();
        keys.iter().filter_map(|k| reg.remove(k)).collect()
    });
    for e in dead {
        stop(&e.player);
        // Слоты сигналов syngui не освобождаются: без сброса PCM остался бы
        // в памяти навсегда. Пустой key отбрасывает декод, ещё идущий в фоне.
        e.signals.key.set_always(String::new());
        e.signals.buf.set_always(None);
        e.signals.playing.set_always(false);
        e.signals.pos.set_always(0.0);
    }
}

/// Запись реестра без аренды — чтобы тест не держал её сам.
#[cfg(all(test, feature = "testing"))]
pub(super) fn inline_peek(sha: &str) -> Option<(AudioSignals, PlayerSlot)> {
    INLINE.with(|reg| {
        reg.borrow()
            .get(sha)
            .map(|e| (e.signals, e.player.clone()))
    })
}
