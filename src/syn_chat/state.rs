//! Реактивное состояние Syn-чата.
//!
//! Inference выполняется in-process через [`crate::syn_chat::session`],
//! generation streaming идёт прямо в сигналы.
//!
//! Вложения живут в трёх местах: черновик текущего сообщения —
//! `pending_attachments`, отправленные — в `ChatMsg.attachments` внутри
//! ленты, байты — в CAS на диске ([`crate::syn_chat::attach::blobs`]).

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use syngui::prelude::*;

pub use crate::agent::state::{
    AttachmentKind, ChatMeta, ChatMsg, ChatMsgKind, ChatMsgRole, MsgAttachment,
};
pub use crate::agent::think_parser::{ThinkParser, ThinkSplit};

use crate::syn_chat::params::SamplingParams;

/// Контекст Syn-чата. Клонируется дёшево (Arc на abort/input_tok_gen + Copy-сигналы).
#[derive(Clone)]
pub struct SynChatCtx {
    /// Список метаданных всех чатов, свежие сверху.
    pub chats: RwSignal<Vec<ChatMeta>>,
    /// id активного чата.
    pub active_chat_id: RwSignal<Option<String>>,
    /// Идёт массовая загрузка (suppress автосейв).
    pub loading: RwSignal<bool>,
    /// Fingerprint последнего сохранённого состояния — для пропуска
    /// идемпотентного автосейва.
    pub last_saved_fp: RwSignal<u64>,

    /// Лента сообщений активного чата.
    pub messages: RwSignal<Vec<ChatMsg>>,
    /// Инкрементальный «хвост» body последнего assistant-плейсхолдера.
    pub streaming_body: RwSignal<String>,
    /// Аналогично — для reasoning/thinking блока.
    pub streaming_thinking: RwSignal<String>,
    /// Live-текст tool-вызова, который модель пишет прямо сейчас (содержимое
    /// `<tool_call>` / ATEM-блока). Показывается карточкой-превью в последнем
    /// assistant-бабле; в ленту не коммитится — его заменяет настоящий
    /// tool_call-бабл, когда вызов дописан.
    pub streaming_tool: RwSignal<String>,
    /// Черновик ввода.
    pub input: RwSignal<String>,
    /// Вложения, прикреплённые к ещё не отправленному сообщению. Уезжают в
    /// `ChatMsg.attachments` при send и очищаются.
    pub pending_attachments: RwSignal<Vec<MsgAttachment>>,
    /// Сколько файлов сейчас обрабатывается (хеширование, ffmpeg, превью).
    /// Ненулевое значение показывает в strip'е плашку «готовим файлы…».
    pub attach_busy: RwSignal<usize>,
    /// Открытый полноэкранный просмотр вложения. `None` — просмотрщик закрыт.
    pub viewer: RwSignal<Option<ViewerState>>,
    /// Agent-loop упёрся в `MAX_AGENT_TURNS` и остановился, не дав
    /// текстового ответа. Взводится воркером, гасится при любом новом
    /// запуске генерации. UI показывает по нему кнопку «Продолжить»:
    /// история цела, надо лишь дать циклу ещё бюджет ходов.
    pub turn_cap_reached: RwSignal<bool>,
    /// Чат, для которого открыт диалог подтверждения архива. `None` —
    /// диалог закрыт. Корзина в шапке и «Закрыть» плитки рейла только
    /// взводят этот сигнал, а `registry::archive` вызывается уже из диалога
    /// — см. `pages::syn_chat::archive_dialog`.
    pub pending_archive: RwSignal<Option<ChatMeta>>,
    /// Поколение поля ввода — для пересоздания editor после очистки.
    pub input_gen: RwSignal<u64>,
    /// Кол-во токенов в `input` (вычисляется debounced'но в фоне).
    pub input_tokens: RwSignal<usize>,
    /// Поколение worker'ов tokenize — для отсева устаревших ответов.
    pub input_tok_gen: Arc<AtomicU64>,
    /// `true` пока идёт генерация (Send disabled).
    pub pending: RwSignal<bool>,
    /// Последняя ошибка для UI.
    pub error: RwSignal<Option<String>>,
    /// Монотонный счётчик прерываний; worker сравнивает со snapshot'ом.
    pub abort: Arc<AtomicU64>,

    /// Параметры sampling (двусторонне связаны с right_panel слайдерами).
    pub params: RwSignal<SamplingParams>,
    /// Системный prompt (зеркало `AppConfig.syn_chat_system_prompt`).
    pub system_prompt: RwSignal<String>,
    /// Состояние раскрытия thinking-блоков по индексу сообщения.
    pub thinking_open: RwSignal<HashMap<usize, bool>>,
    /// Состояние раскрытия свёрнутых групп tool-вызовов (`minimal`-режим).
    /// Ключ — индекс первого `ChatMsg` группы в ленте. Эфемерно, не
    /// persist'ится, сбрасывается при переключении чата.
    pub tool_group_open: RwSignal<HashMap<usize, bool>>,
    /// Раскрытие тела tool-карточки (аргументы вызова / вывод результата).
    /// Ключ — индекс `ChatMsg` в ленте. В `minimal`-режиме управляет
    /// показом тела целиком, в `full` — доразворотом длинного вывода
    /// (см. `pages::syn_chat::message_bubble::TOOL_RESULT_PREVIEW_LINES`).
    pub tool_body_open: RwSignal<HashMap<usize, bool>>,
    /// Раскрытие маркеров компактификации. Ключ — номер итерации
    /// (`CompactionMarker.iteration`, стабилен в отличие от индекса).
    /// Эфемерно, не persist'ится. Дефолт — закрыт.
    pub compaction_open: RwSignal<HashMap<u32, bool>>,
    /// Активный таб правой панели: 0=Параметры, 1=Детали.
    /// См. `context::SYN_RIGHT_PANEL_*`.
    pub right_panel_tab: RwSignal<usize>,
    /// Индекс сообщения, на которое привёл глобальный поиск. Пузырёк с этим
    /// индексом подсвечивается рамкой, пока пользователь не переключит чат
    /// или не кликнет по ленте. Эфемерно, не persist'ится.
    pub highlight_msg: RwSignal<Option<usize>>,

    /// Положение левого разделителя (список чатов ↔ центр). Биндится к
    /// `SplitView::ratio_signal`; drag пишет в сигнал, а
    /// `install_config_autosave` переливает его в
    /// `AppConfig.syn_chat_left_split_ratio`.
    pub left_split_ratio: RwSignal<f32>,
    /// Положение правого разделителя (центр ↔ панель инструментов).
    pub right_split_ratio: RwSignal<f32>,

    // ── статистика последней генерации (для таба «Детали») ──
    pub last_prompt_tokens: RwSignal<u32>,
    pub last_gen_tokens: RwSignal<u32>,
    pub last_prefill_ms: RwSignal<u32>,
    pub last_decode_tps: RwSignal<f32>,
    /// Размер KV-ринга последнего хода в байтах (см. `session::RingPlan`).
    pub kv_cache_bytes: RwSignal<u64>,
    /// Он же в токенах.
    pub last_ring_tokens: RwSignal<u32>,
    /// Сколько ходов agent-loop сделал в последней генерации.
    pub last_turns: RwSignal<u32>,
    /// Сколько токенов контекста влезает в свободную VRAM (потолок ринга).
    pub ctx_budget_tokens: RwSignal<u32>,
    /// Свободная VRAM после хода, МБ.
    pub last_vram_free_mb: RwSignal<u32>,
    /// Сколько токенов промпта взято из префикс-KV (не считалось заново).
    pub last_reused_tokens: RwSignal<u32>,
}

impl SynChatCtx {
    pub fn new() -> Self {
        // Layout-разделители восстанавливаются из persisted-конфига: чтение
        // один раз на создание контекста (сам контекст — синглтон на
        // приложение, см. `crate::run_desktop`).
        let cfg = crate::config::AppConfig::load();
        Self {
            chats: use_signal(Vec::new()),
            active_chat_id: use_signal(None),
            loading: use_signal(false),
            last_saved_fp: use_signal(0),
            messages: use_signal(Vec::new()),
            streaming_body: use_signal(String::new()),
            streaming_thinking: use_signal(String::new()),
            streaming_tool: use_signal(String::new()),
            input: use_signal(String::new()),
            pending_attachments: use_signal(Vec::new()),
            attach_busy: use_signal(0),
            viewer: use_signal(None),
            pending_archive: use_signal(None),
            turn_cap_reached: use_signal(false),
            input_gen: use_signal(0),
            input_tokens: use_signal(0),
            input_tok_gen: Arc::new(AtomicU64::new(0)),
            pending: use_signal(false),
            error: use_signal(None),
            abort: Arc::new(AtomicU64::new(0)),
            params: use_signal(SamplingParams::default()),
            system_prompt: use_signal(String::new()),
            thinking_open: use_signal(HashMap::new()),
            tool_group_open: use_signal(HashMap::new()),
            tool_body_open: use_signal(HashMap::new()),
            compaction_open: use_signal(HashMap::new()),
            right_panel_tab: use_signal(0),
            highlight_msg: use_signal(None),
            left_split_ratio: use_signal(cfg.syn_chat_left_split_ratio),
            right_split_ratio: use_signal(cfg.syn_chat_right_split_ratio),
            last_prompt_tokens: use_signal(0),
            last_gen_tokens: use_signal(0),
            last_prefill_ms: use_signal(0),
            last_decode_tps: use_signal(0.0),
            kv_cache_bytes: use_signal(0),
            last_ring_tokens: use_signal(0),
            last_turns: use_signal(0),
            ctx_budget_tokens: use_signal(0),
            last_vram_free_mb: use_signal(0),
            last_reused_tokens: use_signal(0),
        }
    }

    /// Финализирует assistant-bubble: переливает streaming-сигналы в
    /// последнее сообщение ленты и очищает их. Вызывать только на main
    /// thread (через `run_on_main_thread`).
    pub fn commit_streaming_tail(&self) {
        // Live-превью tool-вызова в ленту не переливается: его либо заменил
        // настоящий tool_call-бабл, либо стрим оборвался посреди вызова.
        if !self.streaming_tool.get_untracked().is_empty() {
            self.streaming_tool.set(String::new());
        }
        let body = self.streaming_body.get_untracked();
        let thinking = self.streaming_thinking.get_untracked();
        let has_any = !body.is_empty() || !thinking.is_empty();
        if !has_any {
            return;
        }
        self.messages.update(|list| {
            // Найти последнее assistant-сообщение.
            if let Some(last) = list.iter_mut().rev().find(|m| m.role == ChatMsgRole::Assistant) {
                last.body.push_str(&body);
                last.thinking.push_str(&thinking);
            }
        });
        self.streaming_body.set(String::new());
        self.streaming_thinking.set(String::new());
    }
}

impl Default for SynChatCtx {
    fn default() -> Self {
        Self::new()
    }
}


/// Состояние полноэкранного просмотрщика вложений.
///
/// Хранит копию списка вложений сообщения, а не индексы в ленту: лента
/// живёт своей жизнью (стриминг дописывает сообщения), а просмотрщик должен
/// уметь листать ровно то, что было открыто.
#[derive(Clone, PartialEq)]
pub struct ViewerState {
    pub items: Vec<MsgAttachment>,
    pub index: usize,
}

impl ViewerState {
    pub fn current(&self) -> Option<&MsgAttachment> {
        self.items.get(self.index)
    }

    /// Листание по кругу — стрелками и колёсиком в просмотрщике.
    pub fn step(&mut self, delta: isize) {
        if self.items.len() < 2 {
            return;
        }
        let len = self.items.len() as isize;
        let next = (self.index as isize + delta).rem_euclid(len);
        self.index = next as usize;
    }
}
