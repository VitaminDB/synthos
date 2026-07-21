//! Подстраница «Аудио модели» — редактор пресетов локальных ASR-моделей.
//!
//! Каждый пресет задаёт параметры [`synaptix::facade::asr::Transcriber`]:
//! движок (Whisper / GigaAM), путь к `.syn` bundle'у,
//! язык-hint, устройство (CPU / GPU auto) и тип данных (F32 / BF16 / F16 / INT8).
//! Запуск инференса локальный — без отдельного llama-server и без HTTP.
//!
//! Загрузка делегируется [`crate::chat::audio::load_selected_model`] —
//! она держит `Transcriber` в `AppCtx.audio.asr`, общий с `chat/audio.rs`
//! (запись с микрофона использует тот же холдер).
//!
//! «Модель по умолчанию» — это `AppCtx.selected_audio_model`: выбор пресета
//! в правой колонке совпадает с тем, что использует runtime для записи.

pub mod audio_models_panel;

use syngui::mgui;
use syngui::prelude::*;

use crate::chat::audio;
use crate::config::{AsrEngineKind, AudioModelConfig};
use crate::context::AppCtx;
use crate::icons::*;

// ─────────────────────────────────────────────────────────────────────────────
// Корневой view
// ─────────────────────────────────────────────────────────────────────────────

pub fn view() -> impl Widget {
    DecoratedBox::new()
        .class("settings-page models-page audio-models-page")
        .child(move || {
            let ctx = use_context::<AppCtx>();
            let models = ctx.audio_models.get();
            let selected = ctx.selected_audio_model.get();

            let idx = selected
                .as_ref()
                .and_then(|name| models.iter().position(|m| &m.name == name));

            let child: Box<dyn Widget> = match idx {
                Some(i) => Box::new(editor(i, models[i].clone())),
                None => Box::new(empty_state()),
            };
            Stack::new().fit(StackFit::Expand).children(vec![child])
        })
}

fn empty_state() -> impl Widget {
    mgui! {
        Center::new() => [
            Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                DecoratedBox::new().class("models-empty-bubble") => [
                    Center::new().child(Icon::new(MI_HEADSET_MIC).class("models-empty-icon")),
                ],
                Text::new("Выберите аудио-модель").class("models-empty-title"),
                Padding::symmetric(32.0, 0.0).child(
                    Text::new("Справа — список ASR-моделей. Нажмите «+», чтобы добавить новую модель распознавания речи. Модель указывается в виде .syn bundle'а — упаковка папки с весами и tokenizer делается командой `syn-pack <dir> -o <out.syn>`.")
                        .class("models-empty-text"),
                ),
            ]
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Редактор
// ─────────────────────────────────────────────────────────────────────────────

fn editor(idx: usize, model: AudioModelConfig) -> impl Widget {
    let name_initial = model.name.clone();
    let path_initial = model.model_path.clone();
    let lang_initial = if model.language.is_empty() {
        "auto".to_string()
    } else {
        model.language.clone()
    };
    let kind_initial = model.kind;
    let device_initial = if model.device.is_empty() {
        "cpu".to_string()
    } else {
        model.device.clone()
    };
    let storage_initial = if model.storage_dtype.is_empty() {
        "f16".to_string()
    } else {
        model.storage_dtype.clone()
    };
    let compute_initial = if model.compute_dtype.is_empty() {
        "f16".to_string()
    } else {
        model.compute_dtype.clone()
    };

    mgui! {
        ScrollView::new().vertical() => [
            Padding::all(32.0) => [
                Column::new().gap(24.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    // ── глобальная секция: микрофон (общая для всех моделей) ──
                    section_card("Микрофон", vec![input_device_row()]),

                    // ── header: имя модели + chip «по умолчанию» + кнопка удалить ──
                    DecoratedBox::new().class("models-card models-card-header") => [
                        Padding::all(20.0) => [
                            Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                                DecoratedBox::new().class("models-header-icon-wrap") => [
                                    Center::new().child(Icon::new(MI_HEADSET_MIC).class("models-header-icon")),
                                ],
                                DecoratedBox::new().class("grow") => [
                                    Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                                        Text::new("Имя модели").class("models-field-label"),
                                        TextField::with_text(name_initial)
                                            .placeholder("Например: Whisper Large v3 Turbo")
                                            .on_change(move |s| {
                                                let s = s.to_string();
                                                let ctx = use_context::<AppCtx>();
                                                let old_name = ctx.audio_models.get_untracked()
                                                    .get(idx).map(|m| m.name.clone());
                                                ctx.audio_models.update(|list| {
                                                    if let Some(m) = list.get_mut(idx) {
                                                        m.name = s.clone();
                                                    }
                                                });
                                                if ctx.selected_audio_model.get_untracked() == old_name {
                                                    ctx.selected_audio_model.set(Some(s));
                                                }
                                            })
                                            .class("models-name-field"),
                                    ]
                                ],
                                default_chip(),
                                Button::new("Удалить")
                                    .icon(MI_DELETE)
                                    .on_click(move || {
                                        let ctx = use_context::<AppCtx>();
                                        let removed_name = ctx.audio_models.get_untracked()
                                            .get(idx).map(|m| m.name.clone());
                                        ctx.audio_models.update(|list| {
                                            if idx < list.len() { list.remove(idx); }
                                        });
                                        if ctx.selected_audio_model.get_untracked() == removed_name {
                                            ctx.selected_audio_model.set(None);
                                        }
                                    })
                                    .class("models-delete-btn"),
                            ]
                        ]
                    ],

                    // ── секция: модель ──
                    section_card("Модель", vec![
                        kind_row(idx, kind_initial),
                        path_row(idx, path_initial),
                        language_row(idx, lang_initial),
                        device_row(idx, device_initial),
                        storage_dtype_row(idx, storage_initial),
                        compute_dtype_row(idx, compute_initial),
                    ]),

                    // ── контрол: загрузка / выгрузка модели + статус ──
                    load_control_card(),
                ]
            ]
        ]
    }
}

/// Chip «по умолчанию» в header'е редактора. Поскольку `editor()` вызывается
/// только для текущего `selected_audio_model`, chip отображается всегда —
/// он подтверждает пользователю, что именно эту модель runtime будет грузить
/// при записи с микрофона.
fn default_chip() -> impl Widget {
    DecoratedBox::new().class("models-active-chip").child(
        Padding::symmetric(10.0, 4.0).child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(MI_CHECK))
                .child(Text::new("по умолчанию")),
        ),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Секции редактора
// ─────────────────────────────────────────────────────────────────────────────

fn kind_row(idx: usize, current: AsrEngineKind) -> Box<dyn Widget> {
    let items = vec![
        DropdownItem::new("whisper", AsrEngineKind::Whisper.display_name()),
        DropdownItem::new("giga_am", AsrEngineKind::GigaAm.display_name()),
    ];
    let current_key = match current {
        AsrEngineKind::Whisper => "whisper",
        AsrEngineKind::GigaAm => "giga_am",
    };
    let control: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(current_key)
            .on_change(move |s| {
                let kind = match s {
                    "whisper" => AsrEngineKind::Whisper,
                    "giga_am" => AsrEngineKind::GigaAm,
                    _ => AsrEngineKind::Whisper,
                };
                let ctx = use_context::<AppCtx>();
                ctx.audio_models.update(|list| {
                    if let Some(m) = list.get_mut(idx) {
                        m.kind = kind;
                    }
                });
            })
            .class("models-active-dropdown"),
    );
    row_frame(
        MI_HEADSET_MIC,
        "Тип движка",
        "Whisper — универсальный многоязычный, GigaAM — заточен под русский.",
        control,
    )
}

fn path_row(idx: usize, initial: String) -> Box<dyn Widget> {
    let field = TextField::with_text(initial)
        .placeholder("/path/to/model.syn")
        .on_change(move |s| {
            let s = s.to_string();
            let ctx = use_context::<AppCtx>();
            ctx.audio_models.update(|list| {
                if let Some(m) = list.get_mut(idx) {
                    m.model_path = s.clone();
                }
            });
        })
        .class("models-path-field");

    let browse = ToolButton::new(MI_FOLDER_OPEN)
        .on_click(move || {
            let dlg = rfd::FileDialog::new().add_filter("Syn model bundle", &["syn"]);
            if let Some(path) = dlg.pick_file() {
                let ctx = use_context::<AppCtx>();
                let s = path.display().to_string();
                ctx.audio_models.update(|list| {
                    if let Some(m) = list.get_mut(idx) {
                        m.model_path = s.clone();
                    }
                });
            }
        })
        .class("models-path-browse");

    let control = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(vec![
            Box::new(field) as Box<dyn Widget>,
            Box::new(browse) as Box<dyn Widget>,
        ]);

    row_frame(
        MI_FOLDER_OPEN,
        "Файл модели (.syn)",
        ".syn bundle с весами и tokenizer — упакуйте папку через `syn-pack <dir> -o <out.syn>`.",
        Box::new(control),
    )
}

/// Dropdown выбора входного аудио-устройства (микрофона).
///
/// Список собирается через `syngui::audio::list_input_devices()` в момент
/// построения row — переключение USB-микрофона видно после ре-открытия
/// вкладки. Виртуальный пункт "auto" вверху означает «дать системе
/// выбрать» (default + перебор).
fn input_device_row() -> Box<dyn Widget> {
    let ctx = use_context::<AppCtx>();
    let current = ctx.general.audio_input_device.get_untracked();
    let current_key = if current.trim().is_empty() {
        "auto".to_string()
    } else {
        current.clone()
    };

    let mut items = vec![DropdownItem::new("auto", "Авто (default + перебор)")];
    for name in syngui::audio::list_input_devices() {
        let label = name.clone();
        items.push(DropdownItem::new(name, label));
    }

    let control: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(current_key)
            .on_change(|s| {
                let s = s.to_string();
                let ctx = use_context::<AppCtx>();
                let normalized = if s == "auto" { String::new() } else { s };
                ctx.general.audio_input_device.set(normalized);
            })
            .class("models-active-dropdown"),
    );

    row_frame(
        MI_HEADSET_MIC,
        "Микрофон",
        "Если выбранное устройство недоступно — система автоматически переключится на работающее и запомнит его.",
        control,
    )
}

fn language_row(idx: usize, current: String) -> Box<dyn Widget> {
    let items = vec![
        DropdownItem::new("auto", "Авто (auto)"),
        DropdownItem::new("ru", "Русский (ru)"),
        DropdownItem::new("en", "Английский (en)"),
        DropdownItem::new("uk", "Украинский (uk)"),
        DropdownItem::new("de", "Немецкий (de)"),
        DropdownItem::new("fr", "Французский (fr)"),
        DropdownItem::new("es", "Испанский (es)"),
        DropdownItem::new("zh", "Китайский (zh)"),
    ];
    let control: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                let s = s.to_string();
                let ctx = use_context::<AppCtx>();
                ctx.audio_models.update(|list| {
                    if let Some(m) = list.get_mut(idx) {
                        m.language = s.clone();
                    }
                });
            })
            .class("models-active-dropdown"),
    );
    row_frame(
        MI_TRANSLATE,
        "Язык распознавания",
        "Hint для Whisper. «Авто» — детектится моделью. GigaAM игнорирует.",
        control,
    )
}

fn device_row(idx: usize, current: String) -> Box<dyn Widget> {
    let items = vec![
        DropdownItem::new("cpu", "CPU"),
        DropdownItem::new("gpu_auto", "GPU (авто: CUDA → Metal → CPU)"),
    ];
    let control: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                let s = s.to_string();
                let ctx = use_context::<AppCtx>();
                ctx.audio_models.update(|list| {
                    if let Some(m) = list.get_mut(idx) {
                        m.device = s.clone();
                    }
                });
            })
            .class("models-active-dropdown"),
    );
    row_frame(
        MI_MEMORY,
        "Устройство",
        "CPU — стабильно везде. GPU (авто) пробует CUDA, затем Metal, иначе откатится на CPU.",
        control,
    )
}

fn storage_dtype_row(idx: usize, current: String) -> Box<dyn Widget> {
    // Storage dtype — формат хранения весов в VRAM. Может отличаться от
    // compute (например storage=fp8e4m3, compute=bf16).
    let items = vec![
        DropdownItem::new("f32", "F32 — без квантизации"),
        DropdownItem::new("bf16", "BF16 — без квантизации"),
        DropdownItem::new("f16", "F16 — без квантизации"),
        DropdownItem::new("q8_0", "Q8_0 — 8-bit (GGML)"),
        DropdownItem::new("q4_0", "Q4_0 — 4-bit (GGML, любая CUDA / CPU)"),
        DropdownItem::new("fp8e4m3", "FP8 E4M3 — native FP8 Tensor Cores (Hopper / Ada / Blackwell)"),
        DropdownItem::new("nvfp4", "NVFP4 — native FP4 Tensor Cores (Blackwell sm_120)"),
    ];
    let control: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                let s = s.to_string();
                let ctx = use_context::<AppCtx>();
                ctx.audio_models.update(|list| {
                    if let Some(m) = list.get_mut(idx) {
                        m.storage_dtype = s.clone();
                    }
                });
            })
            .class("models-active-dropdown"),
    );
    row_frame(
        MI_MEMORY,
        "Storage dtype (веса в VRAM)",
        "Формат хранения весов. F16/BF16/F32 — без квантизации. Q8_0/Q4_0 — GGML (~2-4x VRAM-drop, реальный inference на k_quants kernels). FP8/MXFP4/NF4 — низкоточные форматы. Внимание: после выгрузки модели в VRAM остаётся ~250 MB CUDA driver context + pool — это не утечка, освобождается при выходе из процесса.",
        control,
    )
}

fn compute_dtype_row(idx: usize, current: String) -> Box<dyn Widget> {
    // Compute dtype — strategy выбор для quantized backends:
    // - F32/BF16/F16: eager dequant path (accuracy сохраняется).
    // - FP8/NVFP4: native cuBLASLt path (throughput-win, FP8≈100%, FP4 degraded).
    let items = vec![
        DropdownItem::new("f32", "F32 — максимальная точность"),
        DropdownItem::new("bf16", "BF16 — широкий диапазон, GPU"),
        DropdownItem::new("f16", "F16 — быстрее на GPU (по умолчанию)"),
        DropdownItem::new(
            "fp8e4m3",
            "FP8 E4M3 — native FP8 Tensor Cores (Hopper+/Ada/Blackwell, 2-4× speed)",
        ),
        DropdownItem::new(
            "nvfp4",
            "NVFP4 — native FP4 Tensor Cores (Blackwell sm_120, экспериментально — нужна SmoothQuant)",
        ),
    ];
    let control: Box<dyn Widget> = Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                let s = s.to_string();
                let ctx = use_context::<AppCtx>();
                ctx.audio_models.update(|list| {
                    if let Some(m) = list.get_mut(idx) {
                        m.compute_dtype = s.clone();
                    }
                });
            })
            .class("models-active-dropdown"),
    );
    row_frame(
        MI_BOLT,
        "Compute dtype (активации)",
        "Точность вычислений: активации и matmul accumulator. F16 — компромисс скорость/качество.",
        control,
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Карточка управления загрузкой модели в память (Transcriber)
// ─────────────────────────────────────────────────────────────────────────────

fn load_control_card() -> impl Widget {
    DecoratedBox::new().class("settings-card audio-server-card").child(
        Padding::symmetric(20.0, 16.0).child(move || {
            let ctx = use_context::<AppCtx>();
            let loaded_name = ctx.audio.asr_loaded_name.get();
            let loading = ctx.audio.asr_loading.get();
            let error = ctx.audio.error.get();

            let (status_label, modifier) = if loading {
                ("Загрузка…", "starting")
            } else if loaded_name.is_some() {
                ("Загружена", "running")
            } else if error.is_some() {
                ("Ошибка", "error")
            } else {
                ("Не загружена", "stopped")
            };
            let pill_class = format!("llama-status-pill llama-status-pill-{modifier}");

            let action = action_button(loaded_name.is_some(), loading);

            let info_text = match (&loaded_name, &error, loading) {
                (_, _, true) => "Загружаем модель в память — это может занять до минуты".to_string(),
                (Some(name), _, _) => format!("Активная модель: {name}"),
                (_, Some(err), _) => err.clone(),
                _ => "Нажмите «Загрузить», чтобы инициализировать ASR-движок".to_string(),
            };

            let info_col: Box<dyn Widget> = Box::new(DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(Text::new("ASR движок").class("settings-row-title"))
                    .child(Text::new(info_text).class("settings-row-desc")),
            ));
            let icon_box: Box<dyn Widget> = Box::new(
                DecoratedBox::new()
                    .class("settings-row-icon-wrap")
                    .child(Center::new().child(Icon::new(MI_HEADSET_MIC).class("settings-row-icon"))),
            );
            let pill: Box<dyn Widget> = Box::new(
                DecoratedBox::new().class(pill_class.as_str()).child(
                    Padding::symmetric(10.0, 4.0).child(Text::new(status_label)),
                ),
            );

            Row::new()
                .gap(16.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(vec![icon_box, info_col, pill, action])
        }),
    )
}

fn action_button(loaded: bool, loading: bool) -> Box<dyn Widget> {
    if loading {
        Box::new(
            Button::new("Загрузка…")
                .icon(MI_BOLT)
                .class("models-delete-btn"),
        )
    } else if loaded {
        Box::new(
            Button::new("Выгрузить")
                .icon(MI_STOP)
                .on_click(audio::unload_model)
                .class("models-delete-btn"),
        )
    } else {
        Box::new(
            Button::new("Загрузить")
                .icon(MI_PLAY_ARROW)
                .on_click(audio::load_selected_model)
                .class("models-delete-btn"),
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Локальные хелперы (упрощённые копии из models/mod.rs).
// ─────────────────────────────────────────────────────────────────────────────

fn row_frame(
    icon: &'static str,
    title: &'static str,
    desc: &'static str,
    control: Box<dyn Widget>,
) -> Box<dyn Widget> {
    let inner = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            DecoratedBox::new()
                .class("settings-row-icon-wrap")
                .child(Center::new().child(Icon::new(icon).class("settings-row-icon"))),
        )
        .child(
            DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(Text::new(title).class("settings-row-title"))
                    .child(Text::new(desc).class("settings-row-desc")),
            ),
        )
        .children(vec![control]);

    Box::new(
        DecoratedBox::new().class("settings-row").child(
            Padding::symmetric(24.0, 18.0).child(inner),
        ),
    )
}

fn section_card(title: &'static str, rows: Vec<Box<dyn Widget>>) -> impl Widget {
    let card = DecoratedBox::new().class("settings-card").child(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows),
    );
    Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(title).class("settings-section-title"))
        .child(card)
}
