//! Общая отрисовка полос календаря для месячной сетки и сетки часов:
//! три облика — заливка (событие на весь день, многодневное), подложка с
//! полоской слева (задача доски / Ганта) и «точка + текст» (событие со
//! временем в стиле «плашки»); текст режется по ширине с многоточием и
//! клипом, цвет текста на заливке — по яркости цвета.

use std::sync::Arc;

use syngui::core::canvas::CanvasContext;
use syngui::core::{Color, Point, Rect, Size};
use syngui::mss::{TextAlign, TextDecoration};
use syngui::render::DisplayList;
use syngui::widget::context::TextMeasure;

use super::model::{fmt_hm, EventStyle};

/// Облик полосы.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// Заливка цветом, текст поверх.
    Filled,
    /// Полупрозрачная подложка и полоска слева — внешний элемент.
    Tinted,
    /// Без подложки: точка цвета, время приглушённо, название текстом.
    Plain,
}

impl Look {
    /// Облик по стилю виджета: плашки — заливка для «весь день», точка для
    /// событий со временем; полосы — заливка всем; точки — всем «точка».
    pub fn of(style: EventStyle, is_event: bool, all_day: bool, multi_day: bool) -> Self {
        match style {
            EventStyle::Dot => Look::Plain,
            EventStyle::Bar => {
                if is_event {
                    Look::Filled
                } else {
                    Look::Tinted
                }
            }
            EventStyle::Chip => {
                if !is_event {
                    Look::Tinted
                } else if all_day || multi_day {
                    Look::Filled
                } else {
                    Look::Plain
                }
            }
        }
    }
}

/// Что рисовать в полосе.
pub struct Bar<'a> {
    pub rect: Rect,
    pub color: Color,
    pub look: Look,
    pub title: &'a str,
    /// Время начала — подпись перед названием (однодневные со временем).
    pub time: Option<u32>,
    pub done: bool,
    pub selected: bool,
    pub hover: bool,
    /// Прозрачность всего (перенос — 0.5).
    pub alpha: f32,
    /// Плоский край — полоса продолжается за строку / под «ещё n».
    pub flat_left: bool,
    pub flat_right: bool,
    /// Квадратик «сделано» (заливка) либо точка-переключатель (Plain).
    pub done_box: Option<Rect>,
    pub font: f32,
    /// Цвет текста сетки (для Plain/Tinted) и акцент (рамка выбора).
    pub text: Color,
    pub accent: Color,
}

/// Цвет текста поверх заливки: тёмный на светлых цветах, белый на тёмных.
pub fn text_on(color: Color) -> Color {
    let lum = 0.299 * color.r + 0.587 * color.g + 0.114 * color.b;
    if lum > 0.62 { Color::from_hex("#1F2937") } else { Color::from_hex("#FFFFFF") }
}

/// Ширина строки: измеритель дерева, без него — оценка по кеглю.
pub fn text_width(tm: Option<&Arc<dyn TextMeasure>>, s: &str, font: f32, bold: bool) -> f32 {
    match tm {
        Some(tm) => tm.measure_text_width_styled(s, font, s.chars().count(), bold, None),
        None => s.chars().count() as f32 * font * 0.56,
    }
}

/// Обрезать строку по ширине `max_w` с многоточием.
pub fn ellipsize(tm: Option<&Arc<dyn TextMeasure>>, s: &str, font: f32, bold: bool, max_w: f32) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    if text_width(tm, s, font, bold) <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>().trim_end().to_string() + "…";
        if text_width(tm, &candidate, font, bold) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        return "…".to_string();
    }
    chars[..lo].iter().collect::<String>().trim_end().to_string() + "…"
}

/// Радиусы углов: скруглены только «настоящие» края.
fn radii(flat_left: bool, flat_right: bool, r: f32) -> [f32; 4] {
    let l = if flat_left { 0.0 } else { r };
    let rr = if flat_right { 0.0 } else { r };
    [l, rr, rr, l]
}

/// Нарисовать полосу; `tm` — измеритель для многоточия.
pub fn draw_bar(list: &mut DisplayList, tm: Option<&Arc<dyn TextMeasure>>, b: &Bar) {
    let rect = b.rect;
    if rect.size.width < 2.0 || rect.size.height < 2.0 {
        return;
    }
    let a = b.alpha * if b.done { 0.5 } else { 1.0 };
    let font = (b.font - 1.0).max(7.0);
    let radius = 4.0f32.min(rect.size.height / 2.0);
    let corners = radii(b.flat_left, b.flat_right, radius);
    let color = b.color;
    // Подложка.
    let text_color = match b.look {
        Look::Filled => {
            let fill = if b.hover { 1.0 } else { 0.9 };
            list.push_rect(rect, color.with_alpha(fill * a), corners);
            text_on(color).with_alpha(a)
        }
        Look::Tinted => {
            let tint = if b.hover { 0.3 } else { 0.2 };
            list.push_rect(rect, color.with_alpha(tint * a), corners);
            list.push_rect(
                Rect::new(rect.origin, Size::new(3.0, rect.size.height)),
                color.with_alpha(0.95 * a),
                [corners[0].min(1.5), 0.0, 0.0, corners[3].min(1.5)],
            );
            b.text.with_alpha(0.92 * a)
        }
        Look::Plain => {
            if b.hover || b.selected {
                list.push_rect(rect, b.text.with_alpha(0.07), corners);
            }
            b.text.with_alpha(0.92 * a)
        }
    };
    // Маркер слева: квадратик «сделано» на заливке, точка в Plain.
    let mut text_x = rect.origin.x + if b.look == Look::Tinted { 8.0 } else { 6.0 };
    match (b.look, b.done_box) {
        (Look::Plain, _) => {
            let r = 3.5f32.min(rect.size.height / 4.0);
            let (cx, cy) = (rect.origin.x + 5.0 + r, rect.origin.y + rect.size.height / 2.0);
            let mut k = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
            if b.done {
                k.set_color(color.with_alpha(a));
                k.set_stroke_width(1.5);
                k.draw_arc(cx, cy, r, 0.0, std::f32::consts::TAU);
            } else {
                k.set_color(color.with_alpha(a.max(0.5)));
                k.fill_circle(cx, cy, r);
            }
            k.flush(list);
            text_x = cx + r + 5.0;
        }
        (Look::Filled, Some(bx)) => {
            let on = text_on(color);
            list.push_rect(bx, on.with_alpha(if b.done { 0.9 * a } else { 0.35 * a }), [2.0; 4]);
            if b.done {
                let mut k = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
                k.set_color(color.with_alpha(1.0));
                k.set_stroke_width(1.6);
                k.draw_polyline(&[(bx.origin.x + 2.0, bx.origin.y + 5.0), (bx.origin.x + 4.5, bx.origin.y + 8.0), (bx.origin.x + 8.5, bx.origin.y + 2.5)]);
                k.flush(list);
            }
            text_x = bx.origin.x + bx.size.width + 5.0;
        }
        _ => {}
    }
    // Текст: время приглушённо, название с многоточием, всё под клипом
    // полосы — ничего не вылезает в соседнюю ячейку.
    let right = rect.origin.x + rect.size.width - 4.0;
    if right - text_x < 6.0 {
        return;
    }
    list.push_clip(rect);
    let ty = rect.origin.y + (rect.size.height - font - 2.0) / 2.0;
    let th = font + 4.0;
    let decoration = if b.done { TextDecoration::LineThrough } else { TextDecoration::None };
    if let Some(min) = b.time {
        let label = fmt_hm(min);
        let w = text_width(tm, &label, font, false);
        if right - text_x > w + 12.0 {
            let muted = text_color.with_alpha(text_color.a * 0.7);
            list.push_text_styled_singleline(&label, Rect::new(Point::new(text_x, ty), Size::new(w + 2.0, th)), muted, font, TextAlign::DEFAULT, TextDecoration::None, 500, None);
            text_x += w + 4.0;
        }
    }
    let title = ellipsize(tm, b.title, font, false, right - text_x);
    list.push_text_styled_singleline(&title, Rect::new(Point::new(text_x, ty), Size::new(right - text_x, th)), text_color, font, TextAlign::DEFAULT, decoration, 500, None);
    list.pop_clip();
    if b.selected {
        let mut k = CanvasContext::new(Point::zero(), Size::new(16384.0, 16384.0));
        k.set_color(b.accent);
        k.set_stroke_width(1.5);
        k.draw_rect(rect.origin.x - 1.0, rect.origin.y - 1.0, rect.size.width + 2.0, rect.size.height + 2.0);
        k.flush(list);
    }
}

/// Подпись «ещё n» в ячейке.
pub fn draw_more(list: &mut DisplayList, rect: Rect, n: usize, font: f32, color: Color) {
    let label = syngui::i18n::tr_args("notes.calendar.more", &[("n", &n)]);
    list.push_text_styled_singleline(
        &label,
        Rect::new(Point::new(rect.origin.x + 6.0, rect.origin.y + (rect.size.height - font - 1.0) / 2.0), Size::new((rect.size.width - 8.0).max(4.0), font + 3.0)),
        color,
        (font - 1.0).max(7.0),
        TextAlign::DEFAULT,
        TextDecoration::None,
        600,
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsize_fits_width_and_keeps_short_strings() {
        // Без измерителя — 0.56 кегля на символ: при кегле 10 символ ≈ 5.6 px.
        assert_eq!(ellipsize(None, "короткое", 10.0, false, 100.0), "короткое");
        let cut = ellipsize(None, "Спорт (утро): кардио/силовая по графику", 10.0, false, 60.0);
        assert!(cut.ends_with('…') && cut.chars().count() <= 11, "{cut}");
        assert_eq!(ellipsize(None, "abc", 10.0, false, 0.0), "");
        assert_eq!(ellipsize(None, "abcdef", 10.0, false, 6.0), "…");
        assert_eq!(Look::of(EventStyle::Chip, true, false, false), Look::Plain);
        assert_eq!(Look::of(EventStyle::Chip, true, true, false), Look::Filled);
        assert_eq!(Look::of(EventStyle::Chip, false, true, true), Look::Tinted);
        assert_eq!(Look::of(EventStyle::Bar, true, false, false), Look::Filled);
        assert_eq!(Look::of(EventStyle::Dot, false, true, false), Look::Plain);
        assert_eq!(text_on(Color::from_hex("#FFE066")), Color::from_hex("#1F2937"));
        assert_eq!(text_on(Color::from_hex("#4F8CFF")), Color::from_hex("#FFFFFF"));
    }
}
