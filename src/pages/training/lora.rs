//! Адаптер библиотеки: сведения обучения, включение у модели по умолчанию,
//! удаление.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::{Slider, Toggle};
use synaptixd::{LoraUse, SettingsChange};

use crate::icons::*;

use super::state::{self, Selection, TrainingCtx};

fn line(label: String, value: String) -> Box<dyn Widget> {
    Box::new(mgui! {
        Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Start).class("training-kv") => [
            Text::new(label).class("training-kv-label"),
            DecoratedBox::new().class("grow").child(Text::new(value).max_lines(3).class("training-kv-value")),
        ]
    })
}

/// Набор адаптеров модели по умолчанию с правкой одного.
fn set_default(ctx: TrainingCtx, model: String, name: String, f: impl FnOnce(Option<LoraUse>) -> Option<LoraUse> + Send + 'static) {
    state::act(
        ctx,
        move |c| {
            let entry = c.list()?.into_iter().find(|m| m.id == model).ok_or_else(|| synaptixd::Error::new(synaptixd::ErrorCode::NoModel, model.clone()))?;
            let mut set = entry.settings.loras.clone();
            let cur = set.iter().position(|u| u.name == name).map(|i| set.remove(i));
            if let Some(u) = f(cur) {
                set.push(u);
            }
            c.set(&model, SettingsChange { loras: Some(set), ..Default::default() })
        },
        |_| (),
    );
}

pub fn view(ctx: TrainingCtx, name: String) -> impl Widget {
    let defaults = use_signal(None::<Option<LoraUse>>);
    let n0 = name.clone();
    let body = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(l) = ctx.loras.get().into_iter().find(|l| l.name == n0) else {
            return vec![Box::new(Text::new(tr!("training.lora.missing")).class("training-hint"))];
        };
        // Состояние у модели по умолчанию — из службы.
        if defaults.get_untracked().is_none() {
            if let Some(model) = l.base.clone() {
                let n = l.name.clone();
                std::thread::spawn(move || {
                    let r = synaptixd::Client::connect().and_then(|mut c| c.list()).ok().and_then(|list| list.into_iter().find(|m| m.id == model)).map(|m| m.settings.loras.into_iter().find(|u| u.name == n));
                    syngui::async_runtime::run_on_main_thread(move || defaults.set(Some(r.flatten())));
                });
            }
        }
        let t = &l.train;
        let num = |k: &str| t.get(k).and_then(|v| v.as_f64());
        let mut info = Column::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new(tr!("training.lora.info")).class("training-card-title"))
            .child(line(tr!("training.lora.base"), t.get("base_title").and_then(|v| v.as_str()).map(String::from).or(l.base.clone()).unwrap_or_else(|| l.base_path.clone())))
            .child(line(tr!("training.lora.rank"), format!("r = {}, α = {}", l.rank, l.alpha)))
            .child(line(tr!("training.lora.size"), format!("{:.0} МБ", l.bytes as f64 / 1048576.0)))
            .child(line(tr!("training.lora.path"), crate::paths::pretty(std::path::Path::new(&l.path))));
        if let Some(d) = t.get("dataset").and_then(|v| v.as_str()) {
            info = info.child(line(tr!("training.lora.dataset"), crate::paths::pretty(std::path::Path::new(d))));
        }
        if let (Some(s), Some(tot)) = (num("steps"), num("total_steps")) {
            info = info.child(line(tr!("training.lora.steps"), format!("{s:.0} / {tot:.0}")));
        }
        if let Some(e) = num("eval_loss") {
            info = info.child(line(tr!("training.lora.eval"), format!("{e:.4}")));
        }
        if let Some(e) = num("loss") {
            info = info.child(line(tr!("training.lora.loss"), format!("{e:.4}")));
        }
        if let Some(s) = num("seconds") {
            info = info.child(line(tr!("training.lora.time"), super::form::hms(s)));
        }
        let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().class("training-card").child(info));

        if let Some(model) = l.base.clone() {
            let cur = defaults.get().flatten();
            let on = cur.as_ref().is_some_and(|u| u.enabled);
            let strength = cur.as_ref().map(|u| u.strength).unwrap_or(1.0);
            let (m1, n1, m2, n2) = (model.clone(), l.name.clone(), model.clone(), l.name.clone());
            let toggle = Toggle::with_state(on).on_change(move |v| {
                defaults.set(Some(Some(LoraUse { enabled: v, strength, name: n1.clone() })));
                let n = n1.clone();
                set_default(ctx, m1.clone(), n1.clone(), move |c| Some(LoraUse { enabled: v, ..c.unwrap_or_else(|| LoraUse::on(n)) }));
            }).class("training-toggle");
            // Сила уходит в службу после паузы в движении ползунка.
            let gen = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            let slider = Slider::new().range(0.0, 2.0).step(0.05).value(strength).show_value(2).width(200.0).disabled(!on).on_change(move |v| {
                use std::sync::atomic::Ordering;
                let v = (v * 20.0).round() / 20.0;
                let my = gen.fetch_add(1, Ordering::SeqCst) + 1;
                let (gen, m, n) = (gen.clone(), m2.clone(), n2.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(350));
                    if gen.load(Ordering::SeqCst) != my {
                        return;
                    }
                    syngui::async_runtime::run_on_main_thread(move || {
                        let n0 = n.clone();
                        set_default(ctx, m, n, move |c| Some(LoraUse { strength: v, ..c.unwrap_or_else(|| LoraUse::on(n0)) }));
                    });
                });
            });
            col = col.child(DecoratedBox::new().class("training-card").child(mgui! {
                Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("training.lora.use.title")).class("training-card-title"),
                    Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("grow").child(Text::new(tr!("training.lora.use.default")).max_lines(3).class("training-hint")))
                        .child(toggle),
                    Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("grow").child(Text::new(tr!("training.lora.use.strength")).max_lines(3).class("training-hint")))
                        .child(slider),
                    Text::new(tr!("training.lora.use.chat")).max_lines(4).class("training-hint"),
                ]
            }));
        } else {
            col = col.child(Text::new(tr!("training.lora.no_base")).max_lines(4).class("training-warning"));
        }
        let n3 = l.name.clone();
        col = col.child(mgui! {
            Row::new().gap(10.0) => [
                Button::new(tr!("training.lora.delete")).icon(MI_DELETE).on_click(move || {
                    let n = n3.clone();
                    state::act(ctx, move |c| c.delete_lora(&n), move |_| ctx.selected.set(Selection::Overview));
                }).class("training-btn training-btn-danger"),
            ]
        });
        vec![Box::new(col)]
    });
    let _ = name;
    ScrollView::new().vertical().child(DecoratedBox::new().class("training-center-body").child(body))
}
