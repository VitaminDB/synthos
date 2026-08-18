//! Плавающая аэро-панель «Модели в памяти» — правый край canvas'а
//! нодового редактора.
//!
//! Отвечает на вопрос, который до неё можно было решить только
//! перезапуском приложения: что именно сейчас держит VRAM/RAM и как это
//! отпустить. Источник данных — [`crate::models`]: реестр показывает
//! только живые записи, поэтому в панель не попадает то, что воркер уже
//! отпустил сам.
//!
//! Выгрузка — индивидуальная, кнопкой в строке модели. Кнопка снимает
//! strong-ссылку владельца (hold-слот семейства или слот `NodeRuntime`)
//! и возвращает драйверу резерв CUDA-пула. Если на модель ещё держится
//! активный воркер, строка останется на месте, а в уведомлении будет
//! сказано, что модель занята — врать «выгружено» панель не станет.
//!
//! DOM:
//!   .ne-models-panel[.ne-models-panel--collapsed]
//!     ↳ .ne-models-head       (иконка, заголовок, счётчик, шеврон)
//!     ↳ .ne-models-vram       (живой замер из metrics-сэмплера)
//!     ↳ ScrollView.ne-models-list
//!         ↳ .ne-models-row × N
//!             ↳ .ne-models-row-main (семейство · компонент, файл, устройство)
//!             ↳ .ne-models-unload

use syngui::layout::{CrossAxisAlignment, MainAxisAlignment};
use syngui::prelude::*;
use syngui::widgets::{Column, DecoratedBox, Padding, Reactive, Row, ScrollView, Text, ToolButton};

use crate::context::AppCtx;
use crate::icons::{MI_EXPAND_LESS, MI_EXPAND_MORE, MI_MEMORY, MI_REMOVE_CIRCLE_OUTLINE};
use crate::models::{self, ModelInfo};

/// Панель целиком. Кладётся в canvas-Stack нодового редактора отдельным
/// слоем — см. `mod.rs::canvas_for`.
pub fn view() -> impl Widget {
    let collapsed = use_signal(false);

    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        // Подписка на реестр: загрузка и выгрузка модели перерисовывают
        // панель. Мёртвые записи отсеиваются внутри `list()`.
        if let Some(v) = models::version() {
            let _ = v.get();
        }
        let app = use_context::<AppCtx>();
        let items = models::list();
        let is_collapsed = collapsed.get();

        let mut col = Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![head(items.len(), collapsed, is_collapsed)]);

        if !is_collapsed {
            col = col.children(vec![vram_line()]);
            col = col.children(vec![if items.is_empty() {
                empty_hint()
            } else {
                list(items, &app)
            }]);
        }

        let panel = DecoratedBox::new().child(col).class(if is_collapsed {
            "ne-models-panel ne-models-panel--collapsed"
        } else {
            "ne-models-panel"
        });

        vec![Box::new(panel) as Box<dyn Widget>]
    })
}

/// Шапка: иконка, заголовок, счётчик и шеврон сворачивания. Клик по
/// шеврону — единственный способ убрать панель с глаз; сама она не
/// перехватывает события canvas'а (см. `models_panel_host` в `mod.rs`).
fn head(count: usize, collapsed: RwSignal<bool>, is_collapsed: bool) -> Box<dyn Widget> {
    let chevron = ToolButton::new(if is_collapsed {
        MI_EXPAND_MORE
    } else {
        MI_EXPAND_LESS
    })
    .tooltip(if is_collapsed { "Развернуть" } else { "Свернуть" })
    .on_click(move || collapsed.set(!collapsed.get_untracked()))
    .class("ne-models-chevron");

    let title = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(MI_MEMORY).class("ne-models-head-icon"))
        .child(Text::new("Модели в памяти").class("ne-models-title"))
        .child(Text::new(format!("{count}")).class("ne-models-count"));

    Box::new(
        DecoratedBox::new()
            .child(
                Padding::only(12.0, 8.0, 6.0, 8.0).child(
                    Row::new()
                        .gap(4.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .children(vec![
                            Box::new(title) as Box<dyn Widget>,
                            Box::new(chevron) as Box<dyn Widget>,
                        ]),
                ),
            )
            .class("ne-models-head"),
    )
}

/// Живой замер VRAM из metrics-сэмплера. Своим `Reactive` — чтобы тик
/// сэмплера раз в секунду не пересобирал список моделей под курсором.
fn vram_line() -> Box<dyn Widget> {
    Box::new(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let app = use_context::<AppCtx>();
        if !app.metrics.gpu_available.get() {
            return vec![Box::new(DecoratedBox::new().class("ne-models-vram-empty"))];
        }
        let used = app
            .metrics
            .vram_used_history
            .get()
            .last()
            .unwrap_or(0.0)
            .max(0.0) as u64;
        let total = app.metrics.vram_total.get();
        let text = if total > 0 {
            format!(
                "VRAM {} / {}",
                models::human_bytes(used),
                models::human_bytes(total)
            )
        } else {
            format!("VRAM {}", models::human_bytes(used))
        };
        vec![Box::new(
            Padding::only(12.0, 0.0, 12.0, 8.0).child(Text::new(text).class("ne-models-vram")),
        ) as Box<dyn Widget>]
    }))
}

fn empty_hint() -> Box<dyn Widget> {
    Box::new(
        Padding::only(12.0, 0.0, 12.0, 12.0)
            .child(Text::new("Ничего не загружено").class("ne-models-empty")),
    )
}

fn list(items: Vec<ModelInfo>, app: &AppCtx) -> Box<dyn Widget> {
    let mut col = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch);
    for m in items {
        col = col.children(vec![row(m, app.clone())]);
    }
    Box::new(
        ScrollView::new()
            .vertical()
            .child(Padding::only(8.0, 0.0, 8.0, 8.0).child(col))
            .class("ne-models-list"),
    )
}

/// Строка модели: две текстовые строки слева, кнопка выгрузки справа.
fn row(m: ModelInfo, app: AppCtx) -> Box<dyn Widget> {
    let id = m.id;
    let name = format!("{} · {}", m.family, m.component);
    let meta = if m.bytes > 0 {
        format!("{} · {}", m.device, models::human_bytes(m.bytes))
    } else {
        m.device.clone()
    };

    let main = Column::new()
        .gap(1.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(Text::new(name).class("ne-models-row-title"))
        .child(Text::new(m.label.clone()).class("ne-models-row-label"))
        .child(Text::new(meta).class("ne-models-row-meta"));

    let label_for_msg = m.label.clone();
    let unload = ToolButton::new(MI_REMOVE_CIRCLE_OUTLINE)
        .tooltip("Выгрузить из памяти")
        .on_click(move || {
            if models::unload(id) {
                app.notifications
                    .info(format!("Выгружено: {label_for_msg}"));
            } else {
                app.notifications.info(format!(
                    "{label_for_msg} ещё занята — идёт прогон, освободится по его завершении"
                ));
            }
        })
        .class("ne-models-unload");

    Box::new(
        DecoratedBox::new()
            .child(
                Padding::symmetric(10.0, 8.0).child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .children(vec![
                            Box::new(main) as Box<dyn Widget>,
                            Box::new(unload) as Box<dyn Widget>,
                        ]),
                ),
            )
            .class("ne-models-row"),
    )
}
