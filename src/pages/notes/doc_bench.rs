//! Замер документа заметки: длинная страница в настоящем `DocumentEditor`,
//! полный кадр приложения и счётчики syngui. Нужен, чтобы решать по числам,
//! где именно дорого, — как `syn_chat::feed_bench` для ленты чата. Запуск:
//! `cargo test -p synthos --features testing --lib doc_bench -- --ignored --nocapture --test-threads=1`.

use std::sync::Arc;
use std::time::Instant;

use syngui::core::Point;
use syngui::input::Event;
use syngui::perf::counters::{self, Snapshot};
use syngui::prelude::*;
use syngui::testing::TestHarness;
use syngui::widget::context::TextMeasure;
use syngui::widgets::input::document_editor::{DocLayout, DocumentEditor, DocumentEditorHandle};

const VIEW_W: f32 = 1000.0;
const VIEW_H: f32 = 800.0;

struct Proportional;

impl TextMeasure for Proportional {
    fn measure_text_width(&self, _text: &str, font_size: f32, chars: usize) -> f32 {
        chars as f32 * font_size * 0.55
    }
    fn hit_test_char(&self, text: &str, font_size: f32, x: f32) -> usize {
        ((x / (font_size * 0.55)).round().max(0.0) as usize).min(text.chars().count())
    }
}

/// Страница из `n` блоков: заголовки, абзацы, списки, таблицы и код —
/// как в настоящей длинной заметке.
fn long_note(n: usize) -> String {
    let mut out = String::new();
    for i in 0..n {
        match i % 6 {
            0 => out.push_str(&format!("## Раздел {i}\n\n")),
            1 => out.push_str(&format!(
                "Абзац {i}: текст средней длины, которого хватает на пару строк переноса в колонке страницы.\n\n"
            )),
            2 => out.push_str(&format!("- пункт {i} первый\n- пункт {i} второй\n- пункт {i} третий\n\n")),
            3 => out.push_str(&format!(
                "```rust\nfn block_{i}() -> usize {{\n    let mut acc = 0;\n    for i in 0..{i} {{ acc += i; }}\n    acc\n}}\n```\n\n"
            )),
            4 => out.push_str(&format!(
                "| столбец | значение |\n| --- | --- |\n| строка {i} | {i} |\n| строка {i}б | {} |\n\n",
                i * 2
            )),
            _ => out.push_str(&format!("> Цитата {i} про то, зачем нужен этот раздел.\n\n")),
        }
    }
    out
}

struct Bench {
    h: TestHarness,
    engine: syngui::mss::StyleEngine,
}

impl Bench {
    fn new(md: &str) -> Self {
        let page = DocumentEditorHandle::new();
        let md = md.to_string();
        let mut h = TestHarness::new(Box::new(
            ScrollView::new()
                .vertical()
                .child(move || editor_clone(&page, &md)),
        ));
        h.tree.text_measure = Some(Arc::new(Proportional));
        let engine = h.apply_mss(".grow { flex-grow: 1; }");
        Self { h, engine }
    }

    fn frame(&mut self, label: &str) -> Snapshot {
        let before = counters::snapshot();
        let started = Instant::now();
        let t = self.h.frame(Some(&self.engine), VIEW_W, VIEW_H);
        let wall = started.elapsed().as_secs_f64() * 1000.0;
        let d = counters::snapshot().since(&before);
        println!(
            "{label:<28} wall {wall:7.2} мс | пересб {:5.2} стили {:6.2} раскл {:6.2} отрис {:6.2} | {d} | элементов {}",
            t.rebuild.as_secs_f64() * 1000.0,
            t.styles.as_secs_f64() * 1000.0,
            t.layout.as_secs_f64() * 1000.0,
            t.paint.as_secs_f64() * 1000.0,
            self.h.element_count(),
        );
        d
    }
}

fn editor_clone(page: &DocumentEditorHandle, md: &str) -> DocumentEditor {
    DocumentEditor::new()
        .markdown(md)
        .handle(page)
        .layout(DocLayout::default())
}

#[test]
#[ignore = "замер: cargo test -p synthos --features testing --lib doc_bench -- --ignored --nocapture --test-threads=1"]
fn doc_bench_report() {
    for n in [60usize, 300] {
        let md = long_note(n);
        println!(
            "\n=== заметка на {n} блоков ({:.1} КБ)",
            md.len() as f64 / 1024.0
        );
        let mut b = Bench::new(&md);
        b.frame("открытие");
        b.frame("простой кадр");
        // Ввод текста: щёлкаем в первый абзац и печатаем — редактор
        // перекладывает документ на каждый символ.
        b.h.send_event(&Event::MouseDown {
            button: syngui::input::MouseButton::Left,
            position: Point::new(200.0, 60.0),
        });
        b.h.send_event(&Event::MouseUp {
            button: syngui::input::MouseButton::Left,
            position: Point::new(200.0, 60.0),
        });
        b.frame("клик в текст");
        for i in 0..8 {
            b.h.send_event(&Event::CharInput('ы'));
            if i == 7 {
                b.frame("кадр набора");
            } else {
                b.h.frame(Some(&b.engine), VIEW_W, VIEW_H);
            }
        }

        for i in 0..6 {
            b.h.send_event(&Event::MouseWheel {
                delta: -300.0,
                delta_x: 0.0,
                position: Point::new(VIEW_W / 2.0, VIEW_H / 2.0),
            });
            if i == 5 {
                b.frame("кадр прокрутки");
            } else {
                b.h.frame(Some(&b.engine), VIEW_W, VIEW_H);
            }
        }
    }
}
