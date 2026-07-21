//! Фиксированной ёмкости ring-буфер для истории метрик + превращение
//! в `Vec<DataPoint>` для `LineChart`.
//!
//! Ring даёт O(1) на push/drop_front и стабильный размер памяти.

use std::collections::VecDeque;

use syngui::widgets::charts::DataPoint;

/// Ёмкость по умолчанию для всех историй в `MetricsState`.
/// 300 сэмплов при 1 Гц ≈ 5 минут.
pub const DEFAULT_CAPACITY: usize = 300;

/// Ring-буфер фиксированной ёмкости. При переполнении вытесняет старейший
/// элемент; индексация серии (x) — монотонно растущая, это нужно, чтобы
/// `LineChart` видел сдвиг оси времени, а не «перескок» точек.
#[derive(Debug, Clone)]
pub struct RingBuffer {
    buf: VecDeque<f64>,
    cap: usize,
    /// Абсолютный индекс головы (количество всех когда-либо пушнутых элементов).
    /// Используем для x-координаты, чтобы график визуально «плыл» при новом
    /// сэмпле, а не дёргался.
    head: u64,
}

impl RingBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(cap.max(1)),
            cap: cap.max(1),
            head: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn push(&mut self, v: f64) {
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(v);
        self.head = self.head.saturating_add(1);
    }

    pub fn last(&self) -> Option<f64> {
        self.buf.back().copied()
    }

    /// Максимум по текущему окну (для авто-масштабирования оси Y).
    pub fn max(&self) -> f64 {
        self.buf.iter().copied().fold(0.0_f64, f64::max)
    }

    /// Превратить окно в `Vec<DataPoint>`. X — абсолютный индекс элемента
    /// начиная от `head - len`; Y — значение. При len < 2 возвращаем пусто
    /// (LineChart с одной точкой рисует артефакт).
    pub fn as_series(&self) -> Vec<DataPoint> {
        if self.buf.len() < 2 {
            return Vec::new();
        }
        let start = self.head.saturating_sub(self.buf.len() as u64);
        self.buf
            .iter()
            .enumerate()
            .map(|(i, &y)| DataPoint::new((start + i as u64) as f64, y))
            .collect()
    }
}

impl Default for RingBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_wrap() {
        let mut r = RingBuffer::new(3);
        r.push(1.0);
        r.push(2.0);
        r.push(3.0);
        r.push(4.0);
        assert_eq!(r.len(), 3);
        assert_eq!(r.last(), Some(4.0));
        let pts = r.as_series();
        assert_eq!(pts.len(), 3);
        // X должен быть монотонно растущим и отражать абсолютные индексы
        // — 1,2,3 после того как 0-й элемент вытеснили.
        assert!(pts[1].x > pts[0].x);
        assert_eq!(pts[0].y, 2.0);
        assert_eq!(pts[2].y, 4.0);
    }

    #[test]
    fn single_point_returns_empty_series() {
        let mut r = RingBuffer::new(10);
        r.push(42.0);
        assert!(r.as_series().is_empty());
    }

    #[test]
    fn max_over_window() {
        let mut r = RingBuffer::new(4);
        for v in [5.0, 1.0, 9.0, 3.0] {
            r.push(v);
        }
        assert_eq!(r.max(), 9.0);
        r.push(2.0); // вытесняем 5.0, окно [1,9,3,2] → max всё ещё 9
        assert_eq!(r.max(), 9.0);
        r.push(0.0); // [9,3,2,0]
        r.push(0.0); // [3,2,0,0]
        r.push(0.0); // [2,0,0,0]
        r.push(0.0); // [0,0,0,0]
        assert_eq!(r.max(), 0.0);
    }
}
