//! Документ графика → виджет библиотеки syngui.
//!
//! Пять видов — пять виджетов (`LineChart`, `BarChart`, `PieChart`,
//! `RadarChart`, `GaugeChart`); общее у них только данные документа, а
//! настройки каждый берёт свои. Размер приходит из врезки: класс
//! `notes-chart` задаёт `width/height: 100%`, поэтому график занимает блок
//! целиком, а не свои встроенные умолчания (600×400).
//!
//! Подписи категорий у линий — не свой тип оси, а `format_fn`: значения
//! ложатся на X = 0…n-1, а форматтер возвращает подпись для целых делений.

use syngui::core::Color;
use syngui::prelude::*;
use syngui::widgets::charts::{
    AxisConfig, BarChart, BarMode, BarOrientation, BarSeries, GaugeChart, GaugeSegment, LegendPosition,
    LineChart, PieChart, PieLabelPosition, PieSlice, RadarChart, RadarGridShape, RadarIndicator,
    RadarSeries, Series,
};

use super::model::{fmt_num, ChartDoc, ChartKind, ChartOptions, LegendPos, PieLabels};
use super::ChartHandle;

/// Максимум делений на оси категорий: подписи не должны налезать друг на
/// друга на узком блоке.
const MAX_TICKS: usize = 12;

pub fn view(handle: ChartHandle) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = handle.structure_rev.get();
        vec![build(&handle.lock())]
    })
}

pub fn build(doc: &ChartDoc) -> Box<dyn Widget> {
    match doc.kind {
        ChartKind::Line => Box::new(line(doc)),
        ChartKind::Bar => Box::new(bar(doc)),
        ChartKind::Pie => Box::new(pie(doc)),
        ChartKind::Radar => Box::new(radar(doc)),
        ChartKind::Gauge => Box::new(gauge(doc)),
    }
}

fn legend_pos(o: &ChartOptions) -> LegendPosition {
    match o.legend {
        LegendPos::Top => LegendPosition::Top,
        LegendPos::Bottom => LegendPosition::Bottom,
        LegendPos::Left => LegendPosition::Left,
        LegendPos::Right => LegendPosition::Right,
        LegendPos::None => LegendPosition::None,
    }
}

fn color_of(hex: &str) -> Option<Color> {
    let hex = hex.trim();
    (!hex.is_empty()).then(|| Color::from_hex(hex))
}

/// Ось значений: заголовок, сетка и границы из документа.
fn value_axis(o: &ChartOptions) -> AxisConfig {
    let mut axis = AxisConfig::new().grid(o.grid);
    if !o.y_title.is_empty() {
        axis = axis.title(o.y_title.clone());
    }
    if let Some(v) = o.y_min {
        axis = axis.min(v);
    }
    if let Some(v) = o.y_max {
        axis = axis.max(v);
    }
    axis
}

/// Ось категорий: деления по числу подписей, подпись — из документа.
fn category_axis(doc: &ChartDoc) -> AxisConfig {
    let labels = doc.categories.clone();
    let n = labels.len().max(1);
    let mut axis = AxisConfig::new()
        .grid(doc.options.grid)
        .tick_count(n.min(MAX_TICKS))
        .format(move |v: f64| {
            // Делений может быть больше, чем подписей (шаг оси «красивый»),
            // и попадают они не только в целые — такие остаются без подписи.
            if (v - v.round()).abs() > 1e-6 || v < 0.0 {
                return String::new();
            }
            labels.get(v.round() as usize).cloned().unwrap_or_default()
        });
    if !doc.options.x_title.is_empty() {
        axis = axis.title(doc.options.x_title.clone());
    }
    axis
}

fn line(doc: &ChartDoc) -> impl Widget {
    let o = &doc.options;
    let mut chart = LineChart::new()
        .x_axis(category_axis(doc))
        .y_axis(value_axis(o))
        .legend(legend_pos(o))
        .tooltip(o.tooltip)
        .animate(o.animate)
        .class("notes-chart");
    if !doc.title.is_empty() {
        chart = chart.title(doc.title.clone());
    }
    for s in &doc.series {
        let mut series = Series::new(s.name.clone())
            .data(s.data.iter().enumerate().map(|(i, v)| (i as f64, *v)).collect::<Vec<_>>())
            .smooth(o.smooth)
            .show_points(o.points);
        if o.area > 0.0 {
            series = series.area_fill(o.area);
        }
        if let Some(c) = color_of(&s.color) {
            series = series.color(c);
        }
        chart = chart.series(series);
    }
    chart
}

fn bar(doc: &ChartDoc) -> impl Widget {
    let o = &doc.options;
    let mut chart = BarChart::new()
        .categories(doc.categories.clone())
        .mode(if o.stacked { BarMode::Stacked } else { BarMode::Grouped })
        .orientation(if o.horizontal { BarOrientation::Horizontal } else { BarOrientation::Vertical })
        .x_axis(AxisConfig::new().grid(o.grid))
        .y_axis(value_axis(o))
        .legend(legend_pos(o))
        .tooltip(o.tooltip)
        .animate(o.animate)
        .value_labels(o.value_labels)
        .bar_radius(o.bar_radius)
        .class("notes-chart");
    if !doc.title.is_empty() {
        chart = chart.title(doc.title.clone());
    }
    for s in &doc.series {
        let mut series = BarSeries::new(s.name.clone(), s.data.clone());
        if let Some(c) = color_of(&s.color) {
            series = series.color(c);
        }
        chart = chart.bar_series(series);
    }
    chart
}

fn pie(doc: &ChartDoc) -> impl Widget {
    let o = &doc.options;
    let mut chart = PieChart::new()
        .donut(o.donut)
        .label_position(match o.pie_labels {
            PieLabels::Outside => PieLabelPosition::Outside,
            PieLabels::Inside => PieLabelPosition::Inside,
            PieLabels::None => PieLabelPosition::None,
        })
        .show_percentage(o.percentage)
        .legend(legend_pos(o))
        .tooltip(o.tooltip)
        .animate(o.animate)
        .class("notes-chart");
    if !doc.title.is_empty() {
        chart = chart.title(doc.title.clone());
    }
    let values = doc.series.first();
    for (i, label) in doc.categories.iter().enumerate() {
        let mut slice = PieSlice::new(label.clone(), values.map(|s| s.at(i)).unwrap_or(0.0));
        if let Some(c) = doc.color_at(i).and_then(color_of) {
            slice = slice.color(c);
        }
        chart = chart.slice(slice);
    }
    chart
}

fn radar(doc: &ChartDoc) -> impl Widget {
    let o = &doc.options;
    // Потолок осей общий: разные максимумы у соседних лучей делают фигуру
    // нечитаемой. Без заданного — с запасом над данными.
    let max = o.radar_max.unwrap_or_else(|| (doc.max_value() * 1.15).max(1.0));
    let mut chart = RadarChart::new()
        .grid_shape(if o.radar_circle { RadarGridShape::Circle } else { RadarGridShape::Polygon })
        .grid_levels(o.radar_levels)
        .legend(legend_pos(o))
        .tooltip(o.tooltip)
        .animate(o.animate)
        .class("notes-chart");
    if !doc.title.is_empty() {
        chart = chart.title(doc.title.clone());
    }
    for label in &doc.categories {
        chart = chart.indicator(RadarIndicator::new(label.clone(), max));
    }
    for s in &doc.series {
        let mut series = RadarSeries::new(s.name.clone(), s.data.clone())
            .area_opacity(if o.area > 0.0 { o.area } else { 0.2 })
            .show_points(o.points);
        if let Some(c) = color_of(&s.color) {
            series = series.color(c);
        }
        chart = chart.radar_series(series);
    }
    chart
}

fn gauge(doc: &ChartDoc) -> impl Widget {
    let o = &doc.options;
    let unit = o.unit.clone();
    let mut chart = GaugeChart::new()
        .value(doc.gauge_value())
        .min(o.gauge_min)
        .max(o.gauge_max)
        .needle(o.needle)
        .ticks(o.ticks)
        .labels(o.gauge_labels)
        .animate(o.animate)
        .format(move |v: f64| if unit.is_empty() { fmt_num(v) } else { format!("{} {unit}", fmt_num(v)) })
        .class("notes-chart");
    if !doc.title.is_empty() {
        chart = chart.title(doc.title.clone());
    }
    for z in &o.zones {
        chart = chart.segment(GaugeSegment::new(z.from, z.to, color_of(&z.color).unwrap_or(Color::from_hex("#5470c6"))));
    }
    chart
}
