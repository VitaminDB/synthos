//! Таб «Ламма контроль» — выбор модели + старт/стоп `llama-server`.
//!
//! Реактивно подтягивает пресеты моделей из `AppCtx.models`, статус процесса —
//! из `AppCtx.llama.status_signal`. Единственный источник правды о том,
//! какая модель сейчас выбрана — `AppCtx.selected_model` (он же используется
//! в Настройки → Модели).

use syngui::mgui;
use syngui::prelude::*;
use syngui::StyledWidget;

use crate::config::ModelConfig;
use crate::context::AppCtx;
use crate::icons::*;
use crate::llama::process::GeneralSnapshot;
use crate::llama::ProcessStatus;

pub fn view() -> impl Widget {
    // ВАЖНО: для реактивных блоков НЕ оборачиваем их в DecoratedBox без
    // явного width/height — Reactive даёт Size::zero на первом measure,
    // и такой DecoratedBox коллапсирует. Поэтому ниже Column.child получает
    // «сырое» замыкание: IntoWidget<ReactiveMarker> создаёт Reactive,
    // который меряет по фактическому построенному child’у.
    // Боковые/верхние отступы уже даёт `.right-panel-body`
    // (`padding-left/right/top/bottom` в `right_panel.mss`). Дубль бы удвоил
    // padding — поэтому здесь только вертикальный gap между секциями.
    Column::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(header())
        .child(model_picker())
        .child(model_summary())
        .child(controls())
        .child(super::tools_panel::tools_section())
        .child(super::tools_panel::skills_section())
}

// ─────────────────────────────────────────────────────────────────────────────
// Заголовок + статус-пилюля
// ─────────────────────────────────────────────────────────────────────────────

fn header() -> impl Widget {
    // Header: иконка в квадратике слева + title/subtitle в центре + status pill.
    // Замечание про sizing: pill_box заворачиваем в DecoratedBox
    // с фиксированной шириной, иначе Reactive-ребёнок меряется в 0 и Row
    // «съедает» всю строку.
    let icon_box = DecoratedBox::new()
        .class("llama-header-icon-wrap")
        .child(Center::new().child(Icon::new(MI_MEMORY).class("llama-header-icon")));

    let title_col = Column::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(Text::new("llama-server").class("llama-header-title"))
        .child(Text::new("Локальный сервер").class("llama-header-subtitle"));

    let title_box = DecoratedBox::new().class("grow").child(title_col);

    let pill_box = DecoratedBox::new()
        .class("llama-status-pill-slot")
        .child(status_pill());

    Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon_box)
        .child(title_box)
        .child(pill_box)
}

/// Реактивная пилюля статуса: Row получит это замыкание напрямую и обернёт
/// в `Reactive` через IntoWidget<ReactiveMarker>. Обёртка DecoratedBox
/// вокруг замыкания ломает intrinsic-sizing Row (интересная регрессия —
/// «внешний DecoratedBox + Reactive-ребёнок» измеряется в 0).
fn status_pill() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let s = ctx.llama.status_signal.get();
        let class = format!("llama-pill llama-pill-{}", s.css_modifier());
        DecoratedBox::new().class(class.as_str()).child(
            Center::new().child(Text::new(s.label()).class("llama-pill-text")),
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Дропдаун моделей
// ─────────────────────────────────────────────────────────────────────────────

fn model_picker() -> impl Fn() -> Column + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let models = ctx.models.get();
        let selected = ctx.selected_model.get().unwrap_or_default();

        let child: Box<dyn Widget> = if models.is_empty() {
            Box::new(
                Text::new("Нет пресетов — добавьте в Настройки → Модели")
                    .class("llama-picker-empty"),
            )
        } else {
            let items: Vec<DropdownItem> = models
                .iter()
                .map(|m| {
                    DropdownItem::new(m.name.clone(), m.name.clone()).icon(MI_MEMORY)
                })
                .collect();
            Box::new(
                Dropdown::with_items(items)
                    .selected(selected)
                    .placeholder("Выберите модель")
                    .leading_icon(MI_MEMORY)
                    .on_change(|s| {
                        let ctx = use_context::<AppCtx>();
                        ctx.selected_model.set(Some(s.to_string()));
                    })
                    .class("llama-picker-dropdown"),
            )
        };
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![child])
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Сводка по текущему пресету — что уйдёт в CLI
// ─────────────────────────────────────────────────────────────────────────────

fn model_summary() -> impl Fn() -> StyledWidget<DecoratedBox> + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        // ВАЖНО: здесь используем tracked `.get()`, иначе Reactive не
        // подпишется на смену пресета и сводка не обновится после выбора
        // модели в дропдауне.
        let model = ctx
            .selected_model
            .get()
            .and_then(|name| ctx.models.get().into_iter().find(|m| m.name == name));

        let child: Box<dyn Widget> = match model {
            Some(m) => Box::new(summary_body(&m)),
            None => Box::new(
                Padding::all(16.0).child(
                    Text::new("Выберите пресет, чтобы увидеть аргументы запуска.")
                        .class("llama-summary-hint"),
                ),
            ),
        };
        DecoratedBox::new().class("llama-summary").child(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(vec![child]),
        )
    }
}

fn summary_body(m: &ModelConfig) -> impl Widget {
    let filename = std::path::Path::new(&m.model_path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "— не задан —".to_string());
    let ctx_text = format!("ctx {}", m.ctx_size);
    let params_text = format!("{} доп. параметр(ов)", m.active_params.len());

    Padding::all(14.0).child(mgui! {
        Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_FOLDER_OPEN).class("llama-summary-icon"),
                DecoratedBox::new().class("grow").child(
                    Text::new(filename).class("llama-summary-file"),
                ),
            ],
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(MI_MEMORY).class("llama-summary-icon"),
                Text::new(ctx_text).class("llama-summary-meta"),
                DecoratedBox::new().class("llama-summary-dot"),
                Text::new(params_text).class("llama-summary-meta"),
            ]
        ]
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Кнопки Start / Stop
// ─────────────────────────────────────────────────────────────────────────────

fn controls() -> impl Fn() -> Column + Send + Sync + 'static {
    || {
        let ctx = use_context::<AppCtx>();
        let status = ctx.llama.status_signal.get();
        let can_start = matches!(status, ProcessStatus::Stopped | ProcessStatus::Error)
            && ctx.selected_model.get().is_some();
        let can_stop = matches!(status, ProcessStatus::Starting | ProcessStatus::Running);

        let start = Button::new("Запустить")
            .leading_icon(MI_PLAY_ARROW)
            .disabled(!can_start)
            .on_click(on_start)
            .class("llama-btn-start");

        let stop = Button::new("Остановить")
            .leading_icon(MI_STOP)
            .disabled(!can_stop)
            .on_click(on_stop)
            .class("llama-btn-stop");

        let buttons_row = Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("grow").child(start))
            .child(DecoratedBox::new().class("grow").child(stop));

        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(buttons_row)
            .child(navigate_to_support_hint())
    }
}

fn navigate_to_support_hint() -> impl Widget {
    Button::new("Открыть консоль")
        .leading_icon(MI_TERMINAL)
        .on_click(|| {
            let ctx = use_context::<AppCtx>();
            if ctx.current_route.get_untracked() != "support" {
                ctx.router.lock().unwrap().navigate("support");
                ctx.current_route.set("support".into());
            }
        })
        .class("llama-btn-console")
}

// ─────────────────────────────────────────────────────────────────────────────
// Действия
// ─────────────────────────────────────────────────────────────────────────────

fn on_start() {
    let ctx = use_context::<AppCtx>();
    let Some(model) = selected_model(&ctx) else {
        return;
    };
    let snapshot = build_snapshot(&ctx);

    match ctx.llama.start(snapshot, model) {
        Ok(()) => {}
        Err(e) => {
            // start() сам пушит строку ошибки в логи — логируем в stderr
            // приложения, чтобы при запуске из терминала было видно сразу.
            eprintln!("[synthos] Не удалось запустить llama-server: {e}");
        }
    }
}

fn on_stop() {
    let ctx = use_context::<AppCtx>();
    ctx.llama.stop();
}

fn selected_model(ctx: &AppCtx) -> Option<ModelConfig> {
    let name = ctx.selected_model.get_untracked()?;
    ctx.models.get_untracked().into_iter().find(|m| m.name == name)
}

fn build_snapshot(ctx: &AppCtx) -> GeneralSnapshot {
    GeneralSnapshot {
        server_path: ctx.general.server_path.get_untracked(),
        host: ctx.general.server_host.get_untracked(),
        port: ctx.general.server_port.get_untracked(),
    }
}
