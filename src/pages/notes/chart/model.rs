//! Формат графика: `notes/objects/<id>.chart.json`.
//!
//! Один документ описывает любой из пяти видов библиотеки графиков syngui
//! ([`ChartKind`]): линии, столбцы, круговая, радар и шкала. Данные общие
//! для всех — подписи по горизонтали (`categories`) и ряды значений
//! (`series`); что из этого показывается, решает вид: круговой нужны
//! `categories` + первый ряд (доля на подпись), шкале — одно число из
//! первого ряда, радару — `categories` как оси. Всё остальное живёт в
//! [`ChartOptions`]: поля, которые вид не использует, просто не читаются —
//! так переключение вида не теряет настроек.

use serde::{Deserialize, Serialize};

use super::super::kanban::model::item_id;

/// Вид графика — виджет библиотеки syngui, которым он рисуется.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartKind {
    #[default]
    Line,
    Bar,
    Pie,
    Radar,
    Gauge,
}

impl ChartKind {
    pub const ALL: [ChartKind; 5] =
        [ChartKind::Line, ChartKind::Bar, ChartKind::Pie, ChartKind::Radar, ChartKind::Gauge];

    pub fn key(self) -> &'static str {
        match self {
            ChartKind::Line => "line",
            ChartKind::Bar => "bar",
            ChartKind::Pie => "pie",
            ChartKind::Radar => "radar",
            ChartKind::Gauge => "gauge",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        Self::ALL.iter().copied().find(|k| k.key() == s)
    }

    /// Виду нужен ровно один ряд: круговая делит его на доли, шкала берёт
    /// первое число.
    pub fn single_series(self) -> bool {
        matches!(self, ChartKind::Pie | ChartKind::Gauge)
    }

    /// У вида есть оси со шкалой и сеткой.
    pub fn has_axes(self) -> bool {
        matches!(self, ChartKind::Line | ChartKind::Bar)
    }
}

/// Положение легенды.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LegendPos {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
    None,
}

impl LegendPos {
    pub const ALL: [LegendPos; 5] =
        [LegendPos::Top, LegendPos::Bottom, LegendPos::Left, LegendPos::Right, LegendPos::None];

    pub fn key(self) -> &'static str {
        match self {
            LegendPos::Top => "top",
            LegendPos::Bottom => "bottom",
            LegendPos::Left => "left",
            LegendPos::Right => "right",
            LegendPos::None => "none",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        Self::ALL.iter().copied().find(|k| k.key() == s)
    }
}

/// Подписи долей круговой диаграммы.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PieLabels {
    #[default]
    Outside,
    Inside,
    None,
}

impl PieLabels {
    pub const ALL: [PieLabels; 3] = [PieLabels::Outside, PieLabels::Inside, PieLabels::None];

    pub fn key(self) -> &'static str {
        match self {
            PieLabels::Outside => "outside",
            PieLabels::Inside => "inside",
            PieLabels::None => "none",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        Self::ALL.iter().copied().find(|k| k.key() == s)
    }
}

/// Цветная зона шкалы: от `from` до `to` в единицах значения.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GaugeZone {
    pub from: f64,
    pub to: f64,
    /// `#rrggbb`.
    pub color: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartSeries {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// `#rrggbb`; пусто — цвет по палитре библиотеки.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    #[serde(default)]
    pub data: Vec<f64>,
}

impl ChartSeries {
    pub fn new(name: &str, data: Vec<f64>) -> Self {
        Self { id: item_id("s"), name: name.to_string(), color: String::new(), data }
    }

    /// Значение по индексу категории; чего нет — ноль.
    pub fn at(&self, i: usize) -> f64 {
        self.data.get(i).copied().unwrap_or(0.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartOptions {
    #[serde(default)]
    pub legend: LegendPos,
    #[serde(default = "yes")]
    pub tooltip: bool,
    #[serde(default = "yes")]
    pub animate: bool,
    // ─── Оси (линии и столбцы) ───────────────────────────────────────────
    #[serde(default = "yes")]
    pub grid: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub x_title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub y_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_max: Option<f64>,
    // ─── Линии ───────────────────────────────────────────────────────────
    #[serde(default)]
    pub smooth: bool,
    #[serde(default = "yes")]
    pub points: bool,
    /// Прозрачность заливки под линией, 0 — без заливки.
    #[serde(default)]
    pub area: f32,
    // ─── Столбцы ─────────────────────────────────────────────────────────
    #[serde(default)]
    pub stacked: bool,
    #[serde(default)]
    pub horizontal: bool,
    #[serde(default)]
    pub value_labels: bool,
    #[serde(default)]
    pub bar_radius: f32,
    // ─── Круговая ────────────────────────────────────────────────────────
    /// Доля выреза в середине, 0 — сплошной круг.
    #[serde(default)]
    pub donut: f32,
    #[serde(default)]
    pub pie_labels: PieLabels,
    #[serde(default = "yes")]
    pub percentage: bool,
    // ─── Радар ───────────────────────────────────────────────────────────
    /// Сетка кругами вместо многоугольника.
    #[serde(default)]
    pub radar_circle: bool,
    #[serde(default = "default_levels")]
    pub radar_levels: usize,
    /// Потолок осей; без него — по данным.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radar_max: Option<f64>,
    // ─── Шкала ───────────────────────────────────────────────────────────
    #[serde(default)]
    pub gauge_min: f64,
    #[serde(default = "default_gauge_max")]
    pub gauge_max: f64,
    #[serde(default = "yes")]
    pub needle: bool,
    #[serde(default = "yes")]
    pub ticks: bool,
    #[serde(default = "yes")]
    pub gauge_labels: bool,
    /// Приписка к числу шкалы («%», «₸», «км/ч»).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<GaugeZone>,
}

fn yes() -> bool {
    true
}

fn default_levels() -> usize {
    5
}

fn default_gauge_max() -> f64 {
    100.0
}

impl Default for ChartOptions {
    fn default() -> Self {
        Self {
            legend: LegendPos::default(),
            tooltip: true,
            animate: true,
            grid: true,
            x_title: String::new(),
            y_title: String::new(),
            y_min: None,
            y_max: None,
            smooth: false,
            points: true,
            area: 0.0,
            stacked: false,
            horizontal: false,
            value_labels: false,
            bar_radius: 4.0,
            donut: 0.0,
            pie_labels: PieLabels::default(),
            percentage: true,
            radar_circle: false,
            radar_levels: 5,
            radar_max: None,
            gauge_min: 0.0,
            gauge_max: 100.0,
            needle: true,
            ticks: true,
            gauge_labels: true,
            unit: String::new(),
            zones: Vec::new(),
        }
    }
}

impl ChartOptions {
    pub fn sanitize(&mut self) {
        self.area = self.area.clamp(0.0, 1.0);
        self.bar_radius = self.bar_radius.clamp(0.0, 40.0);
        self.donut = self.donut.clamp(0.0, 0.9);
        self.radar_levels = self.radar_levels.clamp(1, 10);
        if !self.gauge_max.is_finite() || !self.gauge_min.is_finite() {
            self.gauge_min = 0.0;
            self.gauge_max = 100.0;
        }
        if self.gauge_max <= self.gauge_min {
            self.gauge_max = self.gauge_min + 1.0;
        }
        self.zones.retain(|z| z.from.is_finite() && z.to.is_finite() && z.to > z.from);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartDoc {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub kind: ChartKind,
    /// Заголовок над полем графика; пусто — без заголовка.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Подписи по горизонтали: доли круговой, оси радара, деления X.
    #[serde(default)]
    pub categories: Vec<String>,
    /// Цвета категорий — только для круговой; пусто — палитра библиотеки.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<String>,
    #[serde(default)]
    pub series: Vec<ChartSeries>,
    #[serde(default)]
    pub options: ChartOptions,
}

fn default_version() -> u32 {
    1
}

impl ChartDoc {
    /// Новый график с демо-данными: пустой блок графиком не выглядит, а
    /// подставленные числа сразу показывают, что и где правится.
    pub fn template(kind: ChartKind, series_name: &str) -> Self {
        let mut doc = Self {
            version: 1,
            kind,
            title: String::new(),
            categories: Vec::new(),
            colors: Vec::new(),
            series: Vec::new(),
            options: ChartOptions::default(),
        };
        match kind {
            ChartKind::Gauge => {
                doc.series.push(ChartSeries::new(series_name, vec![64.0]));
            }
            ChartKind::Pie => {
                doc.categories = (1..=4).map(|i| format!("{i}")).collect();
                doc.series.push(ChartSeries::new(series_name, vec![40.0, 25.0, 20.0, 15.0]));
            }
            ChartKind::Radar => {
                doc.categories = (1..=5).map(|i| format!("{i}")).collect();
                doc.series.push(ChartSeries::new(series_name, vec![80.0, 60.0, 70.0, 50.0, 90.0]));
            }
            _ => {
                doc.categories = (1..=5).map(|i| format!("{i}")).collect();
                doc.series.push(ChartSeries::new(series_name, vec![12.0, 24.0, 18.0, 30.0, 26.0]));
            }
        }
        doc
    }

    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        let mut doc: Self = serde_json::from_str(json)?;
        doc.sanitize();
        Ok(doc)
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Привести документ в рабочий вид: хотя бы один ряд, числа без NaN,
    /// подписи под длину данных, у круговой и шкалы — один ряд.
    pub fn sanitize(&mut self) {
        self.options.sanitize();
        for s in &mut self.series {
            if s.id.trim().is_empty() {
                s.id = item_id("s");
            }
            for v in &mut s.data {
                if !v.is_finite() {
                    *v = 0.0;
                }
            }
        }
        if self.series.is_empty() {
            self.series.push(ChartSeries::new("", Vec::new()));
        }
        if self.kind.single_series() {
            self.series.truncate(1);
        }
        if self.kind == ChartKind::Gauge {
            if self.series[0].data.is_empty() {
                self.series[0].data.push(0.0);
            }
            self.series[0].data.truncate(1);
            return;
        }
        // Подписи и ряды одной длины — по максимуму из них: обрезать
        // данные молча нельзя (это потеря чисел), а недостающие подписи и
        // пустые точки дорисовываются. Укоротить график — дело
        // [`Self::set_categories`] и `delete_category`: они режут обе
        // стороны разом.
        let longest = self.series.iter().map(|s| s.data.len()).max().unwrap_or(0);
        let n = self.categories.len().max(longest);
        while self.categories.len() < n {
            self.categories.push(format!("{}", self.categories.len() + 1));
        }
        for s in &mut self.series {
            s.data.resize(n, 0.0);
        }
        self.colors.truncate(n);
    }

    pub fn series(&self, id: &str) -> Option<&ChartSeries> {
        self.series.iter().find(|s| s.id == id)
    }

    pub fn series_mut(&mut self, id: &str) -> Option<&mut ChartSeries> {
        self.series.iter_mut().find(|s| s.id == id)
    }

    /// Ряд по id либо по названию (адресация из инструмента агента).
    pub fn find_series(&self, s: &str) -> Option<&ChartSeries> {
        let s = s.trim();
        self.series
            .iter()
            .find(|x| x.id == s)
            .or_else(|| self.series.iter().find(|x| x.name.eq_ignore_ascii_case(s)))
    }

    pub fn add_series(&mut self, name: &str, data: Vec<f64>) -> String {
        let mut s = ChartSeries::new(name, data);
        // Ряд без чисел рисовать нечем — даём ему нули по числу подписей.
        if s.data.is_empty() && !self.categories.is_empty() {
            s.data = vec![0.0; self.categories.len()];
        }
        let id = s.id.clone();
        self.series.push(s);
        self.sanitize();
        id
    }

    pub fn remove_series(&mut self, id: &str) -> bool {
        let n = self.series.len();
        self.series.retain(|s| s.id != id);
        let removed = self.series.len() != n;
        if removed {
            self.sanitize();
        }
        removed
    }

    /// Цвет доли круговой по индексу; пусто — палитра библиотеки.
    pub fn color_at(&self, i: usize) -> Option<&str> {
        self.colors.get(i).map(String::as_str).filter(|c| !c.trim().is_empty())
    }

    pub fn set_color_at(&mut self, i: usize, color: Option<String>) {
        while self.colors.len() <= i {
            self.colors.push(String::new());
        }
        self.colors[i] = color.unwrap_or_default();
        while self.colors.last().is_some_and(|c| c.is_empty()) {
            self.colors.pop();
        }
    }

    /// Значение шкалы — первое число первого ряда.
    pub fn gauge_value(&self) -> f64 {
        self.series.first().map(|s| s.at(0)).unwrap_or(0.0)
    }

    /// Наибольшее значение по всем рядам (потолок осей радара).
    pub fn max_value(&self) -> f64 {
        self.series
            .iter()
            .flat_map(|s| s.data.iter().copied())
            .fold(f64::MIN, f64::max)
            .max(0.0)
    }

    /// Таблица «подписи + ряды» для инструмента агента и чтения глазами.
    pub fn to_table(&self) -> String {
        if self.kind == ChartKind::Gauge {
            return format!("value {}\n", fmt_num(self.gauge_value()));
        }
        let mut out = String::new();
        let names: Vec<&str> = self.series.iter().map(|s| s.name.as_str()).collect();
        out.push_str(&format!("| # | {} |\n", names.join(" | ")));
        for (i, c) in self.categories.iter().enumerate() {
            let row: Vec<String> = self.series.iter().map(|s| fmt_num(s.at(i))).collect();
            out.push_str(&format!("| {c} | {} |\n", row.join(" | ")));
        }
        out
    }

    /// График из markdown-таблицы: первая строка — заголовки рядов, первая
    /// колонка — подписи. Пустые и не-числовые клетки считаются нулями.
    pub fn from_table(md: &str, kind: ChartKind) -> Self {
        let rows: Vec<Vec<String>> = md
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with('|'))
            .map(|l| l.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect())
            .filter(|r: &Vec<String>| !r.iter().all(|c| is_separator(c)))
            .collect();
        let mut doc = Self {
            version: 1,
            kind,
            title: String::new(),
            categories: Vec::new(),
            colors: Vec::new(),
            series: Vec::new(),
            options: ChartOptions::default(),
        };
        let Some((head, body)) = rows.split_first() else {
            doc.sanitize();
            return doc;
        };
        // Первая клетка шапки — подпись колонки подписей, её пропускаем.
        for name in head.iter().skip(1) {
            doc.series.push(ChartSeries::new(name, Vec::new()));
        }
        for row in body {
            doc.categories.push(row.first().cloned().unwrap_or_default());
            for (i, s) in doc.series.iter_mut().enumerate() {
                s.data.push(parse_num(row.get(i + 1).map(String::as_str).unwrap_or("")));
            }
        }
        doc.sanitize();
        doc
    }
}

/// Строка-разделитель markdown-таблицы (`---`, `:--:`).
fn is_separator(cell: &str) -> bool {
    let c = cell.trim();
    !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ')
}

/// Число из клетки: пробелы и разделители тысяч выбрасываются, запятая —
/// десятичная, хвост вроде «%» или «₸» отбрасывается.
pub fn parse_num(s: &str) -> f64 {
    let mut out = String::new();
    for ch in s.trim().chars() {
        match ch {
            '0'..='9' => out.push(ch),
            '-' | '+' if out.is_empty() => out.push(ch),
            '.' | ',' if !out.contains('.') => out.push('.'),
            ' ' | '\u{a0}' | '\'' => {}
            _ => break,
        }
    }
    out.parse().unwrap_or(0.0)
}

/// Числа без хвоста нулей: `12`, `12.5`.
pub fn fmt_num(v: f64) -> String {
    if v.fract().abs() < 1e-9 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Ряд чисел из строки «12, 24, 18» (панель свойств и инструмент агента).
///
/// Запятая — и разделитель списка, и десятичный знак, поэтому разделитель
/// выбирается по строке: есть «;» — режем по нему (и «2,5» остаётся одним
/// числом), нет ни «;», ни «,» — по пробелам.
pub fn parse_values(s: &str) -> Vec<f64> {
    let seps: &[char] = if s.contains(';') {
        &[';', '\n', '\t']
    } else if s.contains(',') {
        &[',', '\n', '\t']
    } else {
        &[' ', '\n', '\t']
    };
    s.split(seps).map(str::trim).filter(|p| !p.is_empty()).map(parse_num).collect()
}

/// Обратно в строку для поля ввода.
pub fn values_text(data: &[f64]) -> String {
    data.iter().map(|v| fmt_num(*v)).collect::<Vec<_>>().join(", ")
}

/// Подписи из строки «Янв, Фев, Мар».
pub fn parse_labels(s: &str) -> Vec<String> {
    s.split([',', '\n', '\t'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_ready_to_draw() {
        let doc = ChartDoc::template(ChartKind::Line, "Ряд 1");
        assert_eq!(doc.categories.len(), doc.series[0].data.len());
        let gauge = ChartDoc::template(ChartKind::Gauge, "Ряд 1");
        assert_eq!(gauge.series[0].data.len(), 1);
    }

    #[test]
    fn sanitize_pads_categories_and_trims_single_series_kinds() {
        let mut doc = ChartDoc::template(ChartKind::Line, "a");
        doc.add_series("b", vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        doc.sanitize();
        assert_eq!(doc.categories.len(), 6, "подписи растут под самый длинный ряд");

        doc.kind = ChartKind::Pie;
        doc.sanitize();
        assert_eq!(doc.series.len(), 1, "круговая держит один ряд");

        doc.kind = ChartKind::Gauge;
        doc.sanitize();
        assert_eq!(doc.series[0].data.len(), 1);
    }

    #[test]
    fn round_trip_through_json() {
        let mut doc = ChartDoc::template(ChartKind::Bar, "Продажи");
        doc.title = "Квартал".to_string();
        doc.options.stacked = true;
        doc.options.legend = LegendPos::Top;
        let back = ChartDoc::parse(&doc.serialize()).expect("parse");
        assert_eq!(back, doc);
    }

    #[test]
    fn table_becomes_categories_and_series() {
        let md = "| Месяц | План | Факт |\n| --- | --- | --- |\n| Янв | 10 | 12,5 |\n| Фев | 20 | 18 |";
        let doc = ChartDoc::from_table(md, ChartKind::Bar);
        assert_eq!(doc.categories, vec!["Янв", "Фев"]);
        assert_eq!(doc.series.len(), 2);
        assert_eq!(doc.series[0].name, "План");
        assert_eq!(doc.series[1].data, vec![12.5, 18.0]);
    }

    #[test]
    fn numbers_survive_units_and_separators() {
        assert_eq!(parse_num("12,5"), 12.5);
        assert_eq!(parse_num("1 200 ₸"), 1200.0);
        assert_eq!(parse_num("-3%"), -3.0);
        assert_eq!(parse_num("нет"), 0.0);
        assert_eq!(values_text(&[1.0, 2.5]), "1, 2.5");
        assert_eq!(parse_values("1, 2.5, 3"), vec![1.0, 2.5, 3.0]);
        assert_eq!(parse_values("1; 2,5; 3"), vec![1.0, 2.5, 3.0], "с «;» запятая остаётся десятичной");
        assert_eq!(parse_values("10 20 30"), vec![10.0, 20.0, 30.0]);
    }

    #[test]
    fn colors_of_categories_shrink_when_cleared() {
        let mut doc = ChartDoc::template(ChartKind::Pie, "a");
        doc.set_color_at(2, Some("#ff0000".into()));
        assert_eq!(doc.colors.len(), 3);
        assert_eq!(doc.color_at(2), Some("#ff0000"));
        doc.set_color_at(2, None);
        assert!(doc.colors.is_empty());
    }
}
