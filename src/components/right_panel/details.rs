//! Таб «Детали» — дашборд метрик llama-server + системы.
//!
//! Читает реактивное состояние `AppCtx.metrics`. Все карточки — `Reactive`
//! замыкания, поэтому автоматически перерисовываются при каждом push в
//! RingBuffer / `set` снимка.
//!
//! Никакого хардкода: все размеры/отступы/цвета — из `details.mss` и
//! `base/variables.mss`. Текст формируется рантайм-хелперами
//! `fmt_bytes`/`fmt_number`/`fmt_rate`, чтобы аккуратно показывать байты
//! и скорости без «съехавших» единиц измерения.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::charts::{
    AxisConfig, BarChart, BarOrientation, BarSeries, DataPoint, LegendPosition, LineChart, Series,
};

use crate::context::AppCtx;
use crate::icons::*;
use crate::metrics::history::RingBuffer;

pub fn view() -> impl Widget {
    let title = DecoratedBox::new()
        .class("details-title-box")
        .child(Text::new("Метрики").class("details-title"));

    ScrollView::new().vertical().child(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(title)
            .child(performance_card)
            .child(tokens_card)
            .child(system_card),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Производительность: tokens/sec (prompt + predicted) + мини-график
// ─────────────────────────────────────────────────────────────────────────────

fn performance_card() -> impl Widget {
    let ctx = use_context::<AppCtx>();
    let timings = ctx.metrics.llama_timings.get();
    let prompt_hist = ctx.metrics.prompt_tps_history.get();
    let predicted_hist = ctx.metrics.predicted_tps_history.get();

    let prompt_rate = timings
        .as_ref()
        .and_then(|t| t.prompt_per_second)
        .unwrap_or(0.0);
    let predicted_rate = timings
        .as_ref()
        .and_then(|t| t.predicted_per_second)
        .unwrap_or(0.0);

    let header: Box<dyn Widget> = Box::new(mgui! {
        Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
            big_metric("Prompt", fmt_rate(prompt_rate), "ток/сек"),
            big_metric("Predicted", fmt_rate(predicted_rate), "ток/сек")
        ]
    });
    let chart = build_tokens_chart(&prompt_hist, &predicted_hist);

    card(
        "Производительность",
        MI_SPEED,
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(vec![header, chart]),
    )
}

fn build_tokens_chart(prompt: &RingBuffer, predicted: &RingBuffer) -> Box<dyn Widget> {
    let prompt_pts = prompt.as_series();
    let predicted_pts = predicted.as_series();
    let any_data = prompt_pts.len() >= 2 || predicted_pts.len() >= 2;

    if !any_data {
        return Box::new(empty_chart_placeholder(
            "история появится после первого ответа",
        ));
    }

    let mut chart = LineChart::new()
        .height(120.0)
        .legend(LegendPosition::None)
        .tooltip(false)
        .animate(false)
        .class("details-mini-chart");

    if prompt_pts.len() >= 2 {
        chart = chart.series(
            Series::new("prompt")
                .data(prompt_pts)
                .color(Color::from_srgb(0x4F, 0x8A, 0xFF, 1.0))
                .line_width(1.5)
                .smooth(true)
                .area_fill(0.18),
        );
    }
    if predicted_pts.len() >= 2 {
        chart = chart.series(
            Series::new("predicted")
                .data(predicted_pts)
                .color(Color::from_srgb(0x3D, 0xD5, 0x98, 1.0))
                .line_width(1.7)
                .smooth(true)
                .area_fill(0.22),
        );
    }

    Box::new(chart)
}

// ─────────────────────────────────────────────────────────────────────────────
// Токены: счётчики
// ─────────────────────────────────────────────────────────────────────────────

fn tokens_card() -> impl Widget {
    let ctx = use_context::<AppCtx>();
    let usage = ctx.metrics.llama_usage.get();
    let timings = ctx.metrics.llama_timings.get();
    let slot = ctx.metrics.llama_slot.get();

    let prompt_tokens = usage.as_ref().and_then(|u| u.prompt_tokens).unwrap_or(0);
    let completion_tokens = usage.as_ref().and_then(|u| u.completion_tokens).unwrap_or(0);
    let total_tokens = usage.as_ref().and_then(|u| u.total_tokens).unwrap_or(0);

    let cache_n = timings.as_ref().and_then(|t| t.cache_n).unwrap_or(0);
    let predicted_n = timings.as_ref().and_then(|t| t.predicted_n).unwrap_or(0);

    let decoded_label = slot
        .as_ref()
        .and_then(|s| s.next_token.n_decoded)
        .map(|v| fmt_number(v as f64))
        .unwrap_or_else(|| "—".into());
    let remain_label = slot
        .as_ref()
        .and_then(|s| s.next_token.n_remain)
        .map(|v| {
            if v < 0 {
                "∞".to_string()
            } else {
                fmt_number(v as f64)
            }
        })
        .unwrap_or_else(|| "—".into());

    card(
        "Токены",
        MI_BOLT_FILLED,
        mgui! {
            Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                metric_row("Prompt",      fmt_number(prompt_tokens as f64)),
                metric_row("Completion",  fmt_number(completion_tokens as f64)),
                metric_row("Total",       fmt_number(total_tokens as f64)),
                metric_row("Cache (hit)", fmt_number(cache_n as f64)),
                metric_row("Predicted",   fmt_number(predicted_n as f64)),
                metric_row("Decoded",     decoded_label),
                metric_row("Remain",      remain_label)
            ]
        },
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Система (объединённая): три кольца GPU / RAM / VRAM, под ними график «Ядра CPU»
// и график GPU-utilization. Если NVML недоступен — кольца GPU/VRAM и график GPU
// заменяются плашкой «GPU метрики недоступны».
// ─────────────────────────────────────────────────────────────────────────────

fn system_card() -> impl Widget {
    let ctx = use_context::<AppCtx>();

    // CPU
    let cpu_per_core = ctx.metrics.cpu_per_core.get();

    // RAM
    let ram_hist = ctx.metrics.ram_used_history.get();
    let ram_total = ctx.metrics.ram_total.get();
    let ram_cur = ram_hist.last().unwrap_or(0.0);
    let ram_ratio = if ram_total > 0 {
        (ram_cur / ram_total as f64) as f32
    } else {
        0.0
    };

    // GPU / VRAM
    let gpu_available = ctx.metrics.gpu_available.get();
    let util_hist = ctx.metrics.gpu_util_history.get();
    let vram_hist = ctx.metrics.vram_used_history.get();
    let vram_total = ctx.metrics.vram_total.get();
    let temp = ctx.metrics.gpu_temperature_c.get();
    let util_cur = util_hist.last().unwrap_or(0.0) as f32;
    let vram_cur = vram_hist.last().unwrap_or(0.0);
    let vram_ratio = if vram_total > 0 {
        (vram_cur / vram_total as f64) as f32
    } else {
        0.0
    };

    let dials: Box<dyn Widget> = if gpu_available {
        Box::new(mgui! {
            Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                dial("GPU", (util_cur / 100.0).clamp(0.0, 1.0), &format!("{:.0}%", util_cur)),
                dial("RAM", ram_ratio.clamp(0.0, 1.0), &fmt_bytes_pair(ram_cur as u64, ram_total)),
                dial("VRAM", vram_ratio.clamp(0.0, 1.0), &fmt_bytes_pair(vram_cur as u64, vram_total))
            ]
        })
    } else {
        Box::new(mgui! {
            Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                dial("RAM", ram_ratio.clamp(0.0, 1.0), &fmt_bytes_pair(ram_cur as u64, ram_total))
            ]
        })
    };

    let cpu_header: Box<dyn Widget> =
        Box::new(Text::new("Ядра CPU").class("details-subheader"));
    let cpu_chart: Box<dyn Widget> = build_cpu_cores_bar(&cpu_per_core);

    let mut children: Vec<Box<dyn Widget>> = vec![dials, cpu_header, cpu_chart];

    if gpu_available {
        let gpu_header: Box<dyn Widget> = Box::new(Text::new("GPU").class("details-subheader"));
        let gpu_chart = build_gpu_chart(&util_hist, &vram_hist, vram_total);
        children.push(gpu_header);
        children.push(gpu_chart);

        if temp > 0 {
            let temp_row: Box<dyn Widget> = Box::new(mgui! {
                Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_THERMOSTAT).class("details-metric-icon"),
                    Text::new(format!("{}°C", temp)).class("details-metric-value")
                ]
            });
            children.push(temp_row);
        }
    } else {
        let notice: Box<dyn Widget> =
            Box::new(DecoratedBox::new().class("details-gpu-unavailable").child(mgui! {
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                    Icon::new(MI_WARNING_AMBER).class("details-gpu-warn-icon"),
                    Text::new("GPU метрики недоступны (NVML не найден)")
                        .class("details-metric-label")
                ]
            }));
        children.push(notice);
    }

    card(
        "Система",
        MI_DEVELOPER_BOARD,
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(children),
    )
}

fn build_cpu_cores_bar(cpu_per_core: &[f32]) -> Box<dyn Widget> {
    if cpu_per_core.is_empty() {
        return Box::new(
            Text::new("Ожидание первых сэмплов…").class("details-metric-label"),
        );
    }

    let categories: Vec<String> = (0..cpu_per_core.len()).map(|i| i.to_string()).collect();
    let values: Vec<f64> = cpu_per_core.iter().map(|v| *v as f64).collect();

    let height = 120.0_f32;

    let chart = BarChart::new()
        .orientation(BarOrientation::Vertical)
        .categories(categories)
        .bar_series(
            BarSeries::new("cpu", values).color(Color::from_srgb(0xEE, 0x5E, 0x48, 1.0)),
        )
        .y_axis(
            AxisConfig::new()
                .min(0.0)
                .max(100.0)
                .tick_count(3)
                .grid(true)
                .axis_line(false)
                .format(|v| format!("{:.0}%", v)),
        )
        .x_axis(AxisConfig::new().grid(false).axis_line(false))
        .legend(LegendPosition::None)
        .tooltip(true)
        .animate(false)
        .value_labels(false)
        .bar_radius(2.0)
        .bar_gap(0.15)
        .height(height)
        .class("details-cpu-cores");

    Box::new(chart)
}

// ─────────────────────────────────────────────────────────────────────────────
// GPU / VRAM (NVML). Используется внутри объединённой карточки `system_card`.
// ─────────────────────────────────────────────────────────────────────────────

fn build_gpu_chart(util: &RingBuffer, vram: &RingBuffer, vram_total: u64) -> Box<dyn Widget> {
    let util_pts = util.as_series();
    let vram_pts: Vec<DataPoint> = if vram_total > 0 {
        vram.as_series()
            .into_iter()
            .map(|p| DataPoint::new(p.x, (p.y / vram_total as f64) * 100.0))
            .collect()
    } else {
        Vec::new()
    };

    if util_pts.len() < 2 && vram_pts.len() < 2 {
        return Box::new(empty_chart_placeholder("собираю данные…"));
    }

    let mut chart = LineChart::new()
        .height(110.0)
        .legend(LegendPosition::None)
        .tooltip(false)
        .animate(false)
        .y_axis(percent_axis())
        .x_axis(hidden_time_axis())
        .class("details-system-chart");

    if util_pts.len() >= 2 {
        chart = chart.series(
            Series::new("gpu")
                .data(util_pts)
                .color(Color::from_srgb(0x3D, 0xD5, 0x98, 1.0))
                .line_width(1.5)
                .smooth(true)
                .area_fill(0.18),
        );
    }
    if vram_pts.len() >= 2 {
        chart = chart.series(
            Series::new("vram")
                .data(vram_pts)
                .color(Color::from_srgb(0xFF, 0xD5, 0x4F, 1.0))
                .line_width(1.5)
                .smooth(true)
                .area_fill(0.18),
        );
    }

    Box::new(chart)
}

/// Общая ось Y «0…100 %» для системных/GPU графиков — фиксированный масштаб
/// не даёт кривой с низкими значениями растягиваться на весь план.
fn percent_axis() -> AxisConfig {
    AxisConfig::new()
        .min(0.0)
        .max(100.0)
        .tick_count(3)
        .grid(true)
        .axis_line(false)
        .format(|v| format!("{:.0}%", v))
}

/// Ось X для time-series: ticks не показываем (абстрактное время), только сетку
/// отключаем — ноль хронологической шкалы только путает.
fn hidden_time_axis() -> AxisConfig {
    AxisConfig::new().grid(false).axis_line(false).tick_count(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Общие «кирпичики»
// ─────────────────────────────────────────────────────────────────────────────

fn card(title: &str, icon: &str, body: impl Widget + 'static) -> impl Widget {
    let title = title.to_string();
    let icon = icon.to_string();
    DecoratedBox::new().class("details-card").child(mgui! {
        Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                Icon::new(icon).class("details-card-icon"),
                Text::new(title).class("details-card-title")
            ],
            body
        ]
    })
}

fn big_metric(label: &str, value: String, unit: &str) -> impl Widget {
    let label = label.to_string();
    let unit = unit.to_string();
    DecoratedBox::new().class("details-big-metric").child(mgui! {
        Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
            Text::new(label).class("details-metric-label"),
            Text::new(value).class("details-metric-big"),
            Text::new(unit).class("details-metric-unit")
        ]
    })
}

fn metric_row(label: &str, value: String) -> impl Widget {
    let label = label.to_string();
    mgui! {
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            DecoratedBox::new().class("grow").child(
                Text::new(label).class("details-metric-label")
            ),
            Text::new(value).class("details-metric-value")
        ]
    }
}

fn dial(label: &str, ratio: f32, caption: &str) -> impl Widget {
    let label = label.to_string();
    let caption = caption.to_string();
    DecoratedBox::new().class("details-dial").child(mgui! {
        Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            CircularProgress::with_value(ratio).class("details-dial-ring"),
            Text::new(label).class("details-dial-label"),
            Text::new(caption).class("details-dial-caption")
        ]
    })
}

fn empty_chart_placeholder(text: &str) -> impl Widget {
    let text = text.to_string();
    DecoratedBox::new()
        .class("details-mini-chart-empty")
        .child(Center::new().child(Text::new(text).class("details-metric-label")))
}

// ─────────────────────────────────────────────────────────────────────────────
// Форматирование
// ─────────────────────────────────────────────────────────────────────────────

fn fmt_rate(v: f64) -> String {
    if v <= 0.0 {
        return "0".into();
    }
    if v >= 100.0 {
        format!("{:.0}", v)
    } else if v >= 10.0 {
        format!("{:.1}", v)
    } else {
        format!("{:.2}", v)
    }
}

fn fmt_number(v: f64) -> String {
    // Пробел-разделитель тысяч без локалей (en-US подход с ','), но по-русски.
    let n = v.abs().round() as u64;
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push('\u{202F}'); // narrow no-break space
        }
        out.push(ch);
    }
    if v < 0.0 {
        out.push('-');
    }
    out.chars().rev().collect()
}

fn fmt_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let n = n as f64;
    if n >= TB {
        format!("{:.2} TB", n / TB)
    } else if n >= GB {
        format!("{:.2} GB", n / GB)
    } else if n >= MB {
        format!("{:.1} MB", n / MB)
    } else if n >= KB {
        format!("{:.0} KB", n / KB)
    } else {
        format!("{} B", n as u64)
    }
}

fn fmt_bytes_pair(used: u64, total: u64) -> String {
    if total == 0 {
        fmt_bytes(used)
    } else {
        format!("{} / {}", fmt_bytes(used), fmt_bytes(total))
    }
}
