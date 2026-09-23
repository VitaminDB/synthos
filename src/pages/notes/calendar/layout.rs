//! Раскладка элементов календаря по строке дней (неделя месячной сетки,
//! ряд «весь день» недели): многодневное событие — одна полоса через
//! ячейки, а не чип на каждый день; дорожки first-fit; обрезка по
//! вместимости ячейки с «ещё n».
//!
//! Чистые функции без геометрии в пикселях: сетки переводят колонки и
//! дорожки в прямоугольники сами.

use std::collections::HashMap;

use super::model::Occurrence;
use super::ExternalItem;

/// Что лежит в отрезке: событие (id) либо внешний элемент (индекс в
/// `GridData::external`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemRef {
    Event(String),
    External(usize),
}

/// Непрерывный отрезок дней одного элемента.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub item: ItemRef,
    pub start: i64,
    pub end: i64,
    /// Минуты начала/конца; `None` — весь день.
    pub time: Option<(u32, u32)>,
}

impl Segment {
    pub fn covers(&self, day: i64) -> bool {
        day >= self.start && day <= self.end
    }

    pub fn multi_day(&self) -> bool {
        self.end > self.start
    }

    pub fn is_event(&self) -> bool {
        matches!(self.item, ItemRef::Event(_))
    }
}

/// Отрезки из вхождений и внешних элементов. Подряд идущие дни одного
/// события склеиваются (`first` начинает новый отрезок — так соседние
/// повторы не сливаются), внешние элементы уже отрезки.
///
/// Порядок — порядок показа внутри дня: многодневные первыми (раньше
/// начались / длиннее — выше, чтобы полосы не «плавали» между строками),
/// затем однодневные: «весь день», по времени, задачи досок после
/// событий. Сортировка стабильна — при равенстве остаётся порядок
/// хранилища.
pub fn segments(occurrences: &[Occurrence], external: &[ExternalItem]) -> Vec<Segment> {
    let mut out: Vec<(u8, Segment)> = Vec::new();
    let mut open: HashMap<String, usize> = HashMap::new();
    for o in occurrences {
        if !o.first {
            if let Some(&i) = open.get(&o.event) {
                if out[i].1.end == o.day - 1 && out[i].1.time == o.time {
                    out[i].1.end = o.day;
                    continue;
                }
            }
        }
        let rank = if o.time.is_none() { 0 } else { 1 };
        out.push((rank, Segment { item: ItemRef::Event(o.event.clone()), start: o.day, end: o.day, time: o.time }));
        open.insert(o.event.clone(), out.len() - 1);
    }
    for (i, e) in external.iter().enumerate() {
        let (day, end_day) = if e.end_day < e.day { (e.end_day, e.day) } else { (e.day, e.end_day) };
        out.push((2, Segment { item: ItemRef::External(i), start: day, end: end_day, time: e.time }));
    }
    out.sort_by(|(ra, a), (rb, b)| {
        let (ma, mb) = (a.multi_day(), b.multi_day());
        mb.cmp(&ma)
            .then_with(|| if ma { a.start.cmp(&b.start).then_with(|| (b.end - b.start).cmp(&(a.end - a.start))) } else { a.start.cmp(&b.start) })
            .then_with(|| ra.cmp(rb))
            .then_with(|| a.time.map(|t| t.0).cmp(&b.time.map(|t| t.0)))
    });
    out.into_iter().map(|(_, s)| s).collect()
}

/// Отрезок, уложенный в строку: колонки `col0..=col1`, дорожка и флаги
/// продолжения за края строки.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub seg: usize,
    pub col0: usize,
    pub col1: usize,
    pub lane: usize,
    pub cont_left: bool,
    pub cont_right: bool,
}

/// Уложить отрезки в строку из `ncols` дней начиная с `row_start`:
/// каждый — на первую дорожку, свободную во всех его колонках.
pub fn pack_row(segs: &[Segment], row_start: i64, ncols: usize) -> Vec<Placed> {
    let row_end = row_start + ncols as i64 - 1;
    let mut lanes: Vec<Vec<bool>> = Vec::new();
    let mut out = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        if s.end < row_start || s.start > row_end {
            continue;
        }
        let col0 = (s.start.max(row_start) - row_start) as usize;
        let col1 = (s.end.min(row_end) - row_start) as usize;
        let lane = (0..=lanes.len()).find(|&l| lanes.get(l).is_none_or(|occ| !occ[col0..=col1].iter().any(|&b| b))).unwrap_or(lanes.len());
        if lane == lanes.len() {
            lanes.push(vec![false; ncols]);
        }
        for c in col0..=col1 {
            lanes[lane][c] = true;
        }
        out.push(Placed { seg: i, col0, col1, lane, cont_left: s.start < row_start, cont_right: s.end > row_end });
    }
    out
}

/// Видимый кусок уложенного отрезка (колонки подряд).
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub placed: usize,
    pub col0: usize,
    pub col1: usize,
}

/// Что показать при `capacity` дорожках на ячейку: в колонке, куда все
/// дорожки влезают, видно всё; иначе последняя дорожка отдаётся под
/// «ещё n», а полосы на ней и ниже прячутся. Возвращает видимые куски
/// (полоса режется там, где соседняя колонка её прячет) и число скрытых
/// по колонкам.
pub fn visible(placed: &[Placed], ncols: usize, capacity: usize) -> (Vec<Run>, Vec<usize>) {
    let capacity = capacity.max(1);
    let mut limit = vec![capacity; ncols];
    let mut hidden = vec![0usize; ncols];
    for c in 0..ncols {
        let deepest = placed.iter().filter(|p| p.col0 <= c && c <= p.col1).map(|p| p.lane + 1).max().unwrap_or(0);
        if deepest > capacity {
            limit[c] = capacity - 1;
        }
        hidden[c] = placed.iter().filter(|p| p.col0 <= c && c <= p.col1 && p.lane >= limit[c]).count();
    }
    let mut runs = Vec::new();
    for (i, p) in placed.iter().enumerate() {
        let mut c = p.col0;
        while c <= p.col1 {
            if p.lane < limit[c] {
                let start = c;
                while c < p.col1 && p.lane < limit[c + 1] {
                    c += 1;
                }
                runs.push(Run { placed: i, col0: start, col1: c });
            }
            c += 1;
        }
    }
    (runs, hidden)
}

/// Дорожек нужно строке (0 — пусто).
pub fn lanes_needed(placed: &[Placed]) -> usize {
    placed.iter().map(|p| p.lane + 1).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::notes::calendar::{ExternalKind, ExternalRef};

    fn occ(id: &str, day: i64, first: bool, time: Option<(u32, u32)>) -> Occurrence {
        Occurrence { event: id.to_string(), day, start: day, time, first, last: false, done: false }
    }

    fn ext(day: i64, end: i64) -> ExternalItem {
        ExternalItem {
            day,
            end_day: end,
            time: None,
            title: "t".into(),
            color: String::new(),
            page: String::new(),
            source: ExternalRef { kind: ExternalKind::Card, object: "b".into(), item: "k".into() },
        }
    }

    #[test]
    fn segments_glue_days_and_order_by_kind() {
        // Трёхдневное событие, ежедневный повтор (first на каждом дне),
        // встреча в 10:00, задача доски на два дня.
        let occs = vec![
            occ("trip", 10, true, None),
            occ("meet", 10, true, Some((600, 660))),
            occ("daily", 10, true, None),
            occ("trip", 11, false, None),
            occ("daily", 11, true, None),
            occ("trip", 12, false, None),
        ];
        let segs = segments(&occs, &[ext(11, 12)]);
        let brief: Vec<(String, i64, i64)> = segs
            .iter()
            .map(|s| (match &s.item { ItemRef::Event(id) => id.clone(), ItemRef::External(i) => format!("ext{i}") }, s.start, s.end))
            .collect();
        assert_eq!(
            brief,
            [
                ("trip".to_string(), 10, 12),
                ("ext0".to_string(), 11, 12),
                ("daily".to_string(), 10, 10),
                ("meet".to_string(), 10, 10),
                ("daily".to_string(), 11, 11),
            ],
            "многодневные первыми (раньше и длиннее — выше), затем весь день, затем по времени; повтор не склеивается"
        );
    }

    #[test]
    fn pack_row_first_fit_and_continuation_flags() {
        let segs = vec![
            Segment { item: ItemRef::Event("a".into()), start: 8, end: 12, time: None },
            Segment { item: ItemRef::Event("b".into()), start: 11, end: 11, time: None },
            Segment { item: ItemRef::Event("c".into()), start: 13, end: 20, time: None },
            Segment { item: ItemRef::Event("far".into()), start: 30, end: 31, time: None },
        ];
        let placed = pack_row(&segs, 10, 7);
        assert_eq!(placed.len(), 3, "отрезок вне строки не укладывается");
        assert_eq!((placed[0].col0, placed[0].col1, placed[0].lane, placed[0].cont_left, placed[0].cont_right), (0, 2, 0, true, false));
        assert_eq!((placed[1].col0, placed[1].col1, placed[1].lane), (1, 1, 1), "день под полосой — второй дорожкой");
        assert_eq!((placed[2].col0, placed[2].col1, placed[2].lane, placed[2].cont_right), (3, 6, 0, true), "свободная дорожка переиспользуется");
        assert_eq!(lanes_needed(&placed), 2);
    }

    #[test]
    fn visible_reserves_last_lane_for_more_only_where_needed() {
        // Колонка 0: четыре элемента, вместимость 3 → две полосы и «ещё 2».
        // Колонка 1: три → всё видно. Полоса на дорожке 2 тянется через
        // обе колонки — в первой прячется, во второй видна.
        let segs = vec![
            Segment { item: ItemRef::Event("a".into()), start: 0, end: 0, time: None },
            Segment { item: ItemRef::Event("b".into()), start: 0, end: 0, time: None },
            Segment { item: ItemRef::Event("c".into()), start: 0, end: 1, time: None },
            Segment { item: ItemRef::Event("d".into()), start: 0, end: 0, time: None },
            Segment { item: ItemRef::Event("e".into()), start: 1, end: 1, time: None },
            Segment { item: ItemRef::Event("f".into()), start: 1, end: 1, time: None },
        ];
        let placed = pack_row(&segs, 0, 2);
        let (runs, hidden) = visible(&placed, 2, 3);
        assert_eq!(hidden, [2, 0]);
        let c = placed.iter().position(|p| segs[p.seg].item == ItemRef::Event("c".into())).unwrap();
        assert_eq!(placed[c].lane, 2, "многодневная полоса легла на дорожку 2 после a и b");
        let c_runs: Vec<(usize, usize)> = runs.iter().filter(|r| r.placed == c).map(|r| (r.col0, r.col1)).collect();
        assert_eq!(c_runs, [(1, 1)], "в колонке 0 спрятана под «ещё», в колонке 1 видна");
        let d = placed.iter().position(|p| segs[p.seg].item == ItemRef::Event("d".into())).unwrap();
        assert!(!runs.iter().any(|r| r.placed == d), "четвёртый элемент скрыт");
        // Вместимости хватает — «ещё» нет.
        let (runs_all, hidden_all) = visible(&placed, 2, 4);
        assert_eq!(hidden_all, [0, 0]);
        assert_eq!(runs_all.len(), placed.len());
    }
}
