//! Панель ввода сообщения.
//!
//! Визуальная структура сохранена: верхняя строка — «pen | editor | sparkle»,
//! разделитель, нижняя строка — toolbar + круглая кнопка отправки. Отличие от
//! предыдущей версии: вместо статичного `Text` используется
//! `MultilineTextEdit`, привязанный к `AppCtx.chat.input`; круглая кнопка
//! реактивна и переключается в «Stop» на время стриминга.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::visual::audio_waveform::AudioWaveform;
use syngui::widgets::MultilineTextEdit;

use crate::chat;
use crate::context::AppCtx;
use crate::icons::*;
use crate::llama::api::LlamaClient;

/// Задержка debounce для запроса `/tokenize` при редактировании ввода.
/// Меньше — спам сервера при быстром наборе; больше — заметная «задержка»
/// точного счётчика. 300 мс — комфортный компромисс.
const TOKENIZE_DEBOUNCE: Duration = Duration::from_millis(300);

pub fn view() -> impl Widget {
    DecoratedBox::new().class("input-panel-wrap").child(mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            // Полоса превью прикреплённых файлов — над input-panel.
            // Реактивная: при пустом draft_attachments схлопывается в zero-size.
            // mgui! автоматически оборачивает Fn-замыкание в Reactive.
            crate::components::attachments_strip::view(),
            DecoratedBox::new().class("input-panel").child(mgui! {
                Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
                        Icon::new(MI_EDIT_NOTE).class("input-pen"),
                        DecoratedBox::new().class("grow").child(editor()),
                        Icon::new(MI_AUTO_AWESOME).class("input-sparkle"),
                    ],
                    waveform_row(),
                    DecoratedBox::new().class("input-divider"),
                    Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).main_axis_alignment(MainAxisAlignment::SpaceBetween) => [
                        Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                            attach_button(),
                            tool(MI_FORMAT_BOLD),
                            tool(MI_FORMAT_ITALIC),
                            tool(MI_FORMAT_UNDERLINED),
                            tool(MI_FORMAT_LIST_BULLETED),
                            tool(MI_FORMAT_LIST_NUMBERED),
                            mic_button_reactive(),
                            crate::components::kb_chip::view(),
                            augment_progress_reactive(),
                            tool(MI_VIDEOCAM),
                        ],
                        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                            token_counter_reactive(),
                            regen_button_reactive(),
                            send_or_stop_button(),
                        ],
                    ],
                ]
            })
        ]
    })
}

/// Многострочный редактор, двусторонне связанный с `chat.input`.
///
/// Внутренний `MultilineTextEdit` не подписан на `chat.input` напрямую —
/// он забирает initial text при создании элемента. Чтобы внешняя очистка
/// (после отправки сообщения в `session::send_message`) визуально
/// сбросила поле, вокруг editor'а стоит реактивная обёртка, подписанная
/// на `chat.input_gen`. При бампе поколения Reactive пересобирает дерево
/// — создаётся новый `MultilineTextEdit` с актуальным `text` (пустым).
fn editor() -> impl Fn() -> syngui::StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let chat = use_context::<AppCtx>().chat.clone();
        let _ = chat.input_gen.get(); // подписка: ребилдим при очистке
        let input = chat.input;
        let input_tokens = chat.input_tokens;
        let initial = input.get_untracked();

        // При сбросе поля (input_gen бампнули) синхронизируем счётчик —
        // пересчитываем эвристику для свежего initial (обычно пустая строка).
        input_tokens.set(estimate_tokens_local(&initial));

        DecoratedBox::new().class("chat-input-field").child(
            MultilineTextEdit::new()
                .text(initial)
                .placeholder("Написать сообщение…")
                .rows(1)
                .max_rows(15)
                .soft_wrap(true)
                .auto_height(true)
                .on_change(move |s| {
                    let text = s.to_string();
                    input.set(text.clone());
                    schedule_tokenize(text);
                })
                .submit_on_enter(true)
                .on_submit(|text| {
                    chat::session::send_message(text.to_string());
                })
                .class("chat-input-edit"),
        )
    }
}

/// Локальная грубая оценка числа токенов — используется мгновенно в
/// `on_change`, чтобы счётчик не мигал и реагировал без задержки. Для
/// UTF-8: ~3 байта на токен (среднее по русскому/английскому для BPE).
/// Сервер через debounce уточнит значение.
fn estimate_tokens_local(s: &str) -> usize {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let bytes = trimmed.len() as f32;
    ((bytes / 3.0).ceil() as usize).max(1)
}

/// Запланировать точную токенизацию текущего `input` через `/tokenize`
/// llama-server. Debounce: запрос выполнится только если за `TOKENIZE_DEBOUNCE`
/// не было новых on_change. Отмена устаревших задач — через монотонный
/// счётчик `input_tok_gen`: каждый on_change бампает, async-воркер сверяет.
///
/// Мгновенно (без ожидания сервера) обновляет `input_tokens` локальной
/// эвристикой — UX не «тормозит» на debounce. Если сервер недоступен,
/// эвристика остаётся финальным значением.
fn schedule_tokenize(text: String) {
    let app = use_context::<AppCtx>();
    let gen_arc: Arc<std::sync::atomic::AtomicU64> = app.chat.input_tok_gen.clone();
    let tokens_sig = app.chat.input_tokens;

    // Бампаем поколение ДО обновления эвристики и старта async:
    // любая предыдущая в полёте задача, увидев новый snapshot, молча уйдёт.
    let snap = gen_arc.fetch_add(1, Ordering::Relaxed).wrapping_add(1);

    // Мгновенный отклик: локальная эвристика.
    tokens_sig.set(estimate_tokens_local(&text));

    // Пустой ввод — сервер дёргать бессмысленно, локальная 0 уже выставлена.
    if text.trim().is_empty() {
        return;
    }

    let host = app.general.server_host.get_untracked();
    let port = app.general.server_port.get_untracked();
    let base_url = format!("http://{}:{}", host, port);

    spawn(async move {
        tokio::time::sleep(TOKENIZE_DEBOUNCE).await;
        // Debounce check — успели ли за это время нажать ещё что-то.
        if gen_arc.load(Ordering::Relaxed) != snap {
            return;
        }

        let client = LlamaClient::with_base_url(base_url);
        let ids = match client.tokenize_text(text).await {
            Ok(ids) => ids,
            // Сервер недоступен / 4xx — оставляем локальную эвристику,
            // уже выставленную выше; молча выходим.
            Err(_) => return,
        };

        if gen_arc.load(Ordering::Relaxed) != snap {
            return;
        }
        let count = ids.len();
        let gen_c = gen_arc.clone();
        run_on_main_thread(move || {
            if gen_c.load(Ordering::Relaxed) != snap {
                return;
            }
            let app = use_context::<AppCtx>();
            app.chat.input_tokens.set(count);
        });
    });
}

/// Компактный чип «N токенов» слева от кнопки отправки. Реактивно
/// подписан на `input_tokens`. Когда пусто — скрывается (возвращает
/// zero-size DecoratedBox), чтобы не занимать место и не «светить»
/// нулём до первого символа.
fn token_counter_reactive(
) -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let chat = use_context::<AppCtx>().chat.clone();
        let n = chat.input_tokens.get();

        if n == 0 {
            return DecoratedBox::new().class("input-token-chip-empty");
        }

        let label = format!("{} ток.", fmt_token_number(n));
        DecoratedBox::new().class("input-token-chip").child(mgui! {
            Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_BOLT_FILLED).class("input-token-chip-icon"),
                Text::new(label).class("input-token-chip-text")
            ]
        })
    }
}

/// Форматирование с узким неразрывным пробелом между разрядами тысяч —
/// тот же приём, что в `chat_header::fmt_thin_number`, но не тянем его
/// сюда, чтобы не плодить pub-экспортов поперёк модулей.
fn fmt_token_number(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push('\u{202F}');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

/// Круглая кнопка в правом нижнем углу. Реактивно переключается между:
/// - Send (по умолчанию): отправляет текущий `chat.input` через `chat::session`;
/// - Stop (во время `pending`): аборт активного стрима.
///
/// Отсутствие активного чата не блокирует Send: `send_message` сам создаёт
/// новый чат через `registry::create_new`. Это естественнее, чем просить
/// пользователя нажимать «+» перед первым сообщением.
/// Кнопка «Сгенерировать заново» в input-panel рядом с send/stop. Удобна,
/// когда последнее сообщение — assistant-tool без последующего text-ответа,
/// и наводить курсор на конкретный bubble неудобно. Поведение идентично
/// regen-кнопке в bubble: вызывает `regenerate_from(last_msg_idx)`, который
/// сам найдёт ближайший предыдущий user-Text и обрежет хвост.
///
/// Скрыта в трёх случаях (zero-size DecoratedBox):
/// - `chat.pending == true` — генерация уже идёт;
/// - в ленте нет ни одного user-Text сообщения (нечего перегенерировать);
/// - в ленте после последнего user-Text не появилось ничего нового
///   (только что отправили — pending=true в этот момент тоже, так что
///   условие избыточно, но оставляем явно).
fn regen_button_reactive(
) -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let chat = use_context::<AppCtx>().chat.clone();
        // Подписка ТОЛЬКО на `pending` и `active_chat_id` — оба меняются
        // редко (старт/конец стрима, переключение чата). На `messages`
        // подписываться нельзя: сигнал обновляется на каждом SSE-чанке во
        // время стрима, и Reactive пересобирался бы десятки раз в секунду —
        // это топило main-поток и ломало overlay-регистрацию (dropdown'ы
        // переставали ловить hover/click). messages читаем untracked.
        let pending = chat.pending.get();
        let _ = chat.active_chat_id.get();
        if pending {
            return DecoratedBox::new().class("input-regen-empty");
        }
        let msgs = chat.messages.get_untracked();
        if msgs.is_empty() {
            return DecoratedBox::new().class("input-regen-empty");
        }
        let last_idx = msgs.len() - 1;
        if chat::session::find_last_user_before(&msgs, last_idx).is_none() {
            return DecoratedBox::new().class("input-regen-empty");
        }
        DecoratedBox::new().class("input-regen-wrap").child(
            ToolButton::new(MI_AUTORENEW)
                .tooltip("Сгенерировать заново")
                .on_click(move || chat::session::regenerate_from(last_idx))
                .class("input-regen"),
        )
    }
}

fn send_or_stop_button() -> impl Fn() -> StyledWidget<ToolButton> + Send + Sync + 'static {
    || {
        let chat = use_context::<AppCtx>().chat.clone();
        if chat.pending.get() {
            ToolButton::new(MI_STOP)
                .on_click(chat::session::abort)
                .class("input-send input-send-stop")
        } else {
            ToolButton::new(MI_SEND)
                .on_click(|| {
                    let input = use_context::<AppCtx>().chat.input;
                    chat::session::send_message(input.get_untracked());
                })
                .class("input-send")
        }
    }
}

fn tool(icon: &'static str) -> impl Widget {
    ToolButton::new(icon).class("input-tool")
}

/// Реактивный индикатор «идёт RAG-поиск в KB» (auto-augment pre-step).
/// Когда `kb.augment_in_progress=false` — возвращает zero-size DecoratedBox
/// (класс `.input-kb-progress-empty`); во время поиска — Row с
/// hourglass-иконкой и текстом «Поиск в KB…», pulse-анимация задаётся в MSS.
fn augment_progress_reactive(
) -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        if !app.kb.augment_in_progress.get() {
            return DecoratedBox::new().class("input-kb-progress-empty");
        }
        DecoratedBox::new().class("input-kb-progress").child(mgui! {
            Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_HOURGLASS_TOP).class("input-kb-progress-icon"),
                Text::new("Поиск в KB…").class("input-kb-progress-text"),
            ]
        })
    }
}

/// Кнопка «прикрепить файл». Открывает нативный диалог выбора картинок,
/// добавляет их в `chat.draft_attachments`. Лента превью над input
/// реактивно отображает результат.
fn attach_button() -> impl Widget {
    ToolButton::new(MI_ATTACH_FILE)
        .on_click(|| crate::chat::attach::pick_and_attach())
        .class("input-tool")
}

/// Реактивная кнопка микрофона. Toggle: клик — старт записи (иконка
/// MI_MIC → MI_STOP, добавляется класс `.input-mic-recording` для
/// pulse-анимации); повторный клик — стоп + транскрипция.
///
/// Во время `transcribing.get()` иконка переключается на
/// MI_HOURGLASS_TOP и кнопка визуально выделяется (`.input-mic-busy`),
/// чтобы пользователь видел, что результат вот-вот придёт.
fn mic_button_reactive() -> impl Fn() -> StyledWidget<ToolButton> + Send + Sync + 'static {
    || {
        let actx = use_context::<AppCtx>().audio.clone();
        let recording = actx.is_recording.get();
        let transcribing = actx.transcribing.get();

        let (icon, class, tooltip) = if transcribing {
            (
                MI_HOURGLASS_TOP,
                "input-tool input-mic-busy",
                "Распознавание…",
            )
        } else if recording {
            (
                MI_STOP,
                "input-tool input-mic-recording",
                "Остановить запись и распознать",
            )
        } else {
            (
                MI_MIC,
                "input-tool",
                "Записать голос (распознать через ASR)",
            )
        };

        ToolButton::new(icon)
            .tooltip(tooltip)
            .on_click(move || {
                // Во время transcribing клик игнорируется — нет смысла
                // запускать новую запись поверх ожидания результата.
                if transcribing {
                    return;
                }
                crate::chat::audio::toggle_recording();
            })
            .class(class)
    }
}

/// Контейнер с waveform-визуализацией. Реактивно подписан на
/// `audio.is_recording` и `audio.vis_handle`. Пока запись не идёт —
/// возвращает пустой DecoratedBox с классом `.input-waveform-empty`
/// (height: 0 в MSS), чтобы не добавлять вертикальный зазор.
///
/// Если открыто глобальное FAB-окно распознавания (`voice.panel_open=true`),
/// тоже возвращаем empty: запись идёт «там», waveform отображается в
/// панели FAB (через `voice_fab::aura`), а не здесь — иначе под input panel
/// появлялся бы паразитный мини-waveform параллельно с большой аурой.
fn waveform_row() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let app = use_context::<AppCtx>();
        let actx = app.audio.clone();
        let _ = actx.is_recording.get();
        let handle = actx.vis_handle.get();
        let voice_open = app.voice.panel_open.get();

        if voice_open {
            return DecoratedBox::new().class("input-waveform-empty");
        }

        match handle {
            Some(h) => DecoratedBox::new().class("input-waveform-wrap").child(
                AudioWaveform::new(h)
                    .bars(56)
                    .height(36.0)
                    .class("input-waveform"),
            ),
            None => DecoratedBox::new().class("input-waveform-empty"),
        }
    }
}
