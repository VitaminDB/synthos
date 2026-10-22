//! Форма новой задачи обучения.
//!
//! Форма строится один раз; поля пишут в `TrainingCtx.form`, перестраиваются
//! только зависимые куски (адаптеры для продолжения — от модели, итог
//! проверки датасета). Иначе поле имени теряло бы фокус на каждой букве.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::{Dropdown, DropdownItem, SpinBox, TextField, Toggle};

use crate::icons::*;
use crate::syn_chat::SynModelRegistry;

use super::state::{self, Check, JobForm, Selection, TrainingCtx};

/// Открыть форму: модель — текущая модель чата или первая, датасет — первый.
pub fn open_new(ctx: TrainingCtx) {
    let mut f = ctx.form.get_untracked();
    let reg = use_context::<SynModelRegistry>();
    let models: Vec<_> = reg.models.get_untracked().into_iter().filter(|m| m.trainable()).collect();
    if f.model.as_ref().is_none_or(|m| !models.iter().any(|x| &x.id == m)) {
        let cur = reg.current.get_untracked().and_then(|c| models.iter().find(|m| m.path == c.path).map(|m| m.id.clone()));
        let def = reg.default_id.get_untracked().filter(|d| models.iter().any(|m| &m.id == d));
        f.model = cur.or(def).or_else(|| models.first().map(|m| m.id.clone()));
    }
    let datasets = ctx.datasets.get_untracked();
    if f.dataset.as_ref().is_none_or(|d| !datasets.iter().any(|x| std::path::Path::new(&x.path) == d)) {
        f.dataset = datasets.first().map(|d| std::path::PathBuf::from(&d.path));
    }
    if f.name.is_empty() {
        if let Some(m) = &f.model {
            f.name = state::suggest_name(m, f.dataset.as_deref());
        }
    }
    ctx.form.set(f);
    ctx.check.set(Check::None);
    ctx.selected.set(Selection::NewJob);
}

fn update(ctx: TrainingCtx, f: impl FnOnce(&mut JobForm)) {
    let mut form = ctx.form.get_untracked();
    f(&mut form);
    if ctx.form.get_untracked() != form {
        ctx.form.set(form);
        ctx.check.set(Check::None);
    }
}

fn field(label: String, hint: String, control: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(mgui! {
        Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center).class("training-field") => [
            DecoratedBox::new().class("grow").child(mgui! {
                Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(label).class("training-field-label"),
                    Text::new(hint).max_lines(3).class("training-field-hint"),
                ]
            }),
            control,
        ]
    })
}

fn choice(width: f32, items: Vec<(String, String)>, selected: String, on: impl FnMut(&str) + Send + 'static) -> Dropdown {
    let mut dd = Dropdown::new().width(width);
    for (id, label) in items {
        dd = dd.item(DropdownItem::new(id, label));
    }
    dd.selected(selected).on_change(on)
}

pub fn view(ctx: TrainingCtx) -> impl Widget {
    let f = ctx.form.get_untracked();
    let reg = use_context::<SynModelRegistry>();
    let name_rev = use_signal(0u64);

    // Модель и датасет: списки приходят из службы асинхронно — дропдауны
    // перестраиваются по ним; смена подставляет имя адаптера, если его не правили.
    let model_dd = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let list: Vec<_> = reg.models.get().into_iter().filter(|m| m.trainable()).collect();
        let mut cur = ctx.form.get_untracked().model;
        if cur.as_ref().is_none_or(|m| !list.iter().any(|x| &x.id == m)) {
            cur = list.first().map(|m| m.id.clone());
            if let Some(m) = cur.clone() {
                update(ctx, |x| {
                    if x.name.is_empty() {
                        x.name = state::suggest_name(&m, x.dataset.as_deref());
                    }
                    x.model = Some(m);
                });
                name_rev.set(name_rev.get_untracked() + 1);
            }
        }
        let models: Vec<(String, String)> = list.iter().map(|m| (m.id.clone(), m.title.clone())).collect();
        vec![Box::new(choice(320.0, models, cur.unwrap_or_default(), move |v: &str| {
            let v = v.to_string();
            let old = ctx.form.get_untracked();
            let auto = old.name.is_empty() || old.model.as_ref().is_some_and(|m| old.name == state::suggest_name(m, old.dataset.as_deref()));
            update(ctx, |x| {
                x.model = Some(v.clone());
                x.resume = None;
                if auto {
                    x.name = state::suggest_name(&v, x.dataset.as_deref());
                }
            });
            if auto {
                name_rev.set(name_rev.get_untracked() + 1);
            }
        }))]
    });
    let ds_dd = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let list = ctx.datasets.get();
        let mut cur = ctx.form.get_untracked().dataset;
        if cur.as_ref().is_none_or(|d| !list.iter().any(|x| std::path::Path::new(&x.path) == d)) {
            cur = list.first().map(|d| std::path::PathBuf::from(&d.path));
            if let Some(p) = cur.clone() {
                let old = ctx.form.get_untracked();
                let auto = old.name.is_empty() || old.model.as_ref().is_some_and(|m| old.name == state::suggest_name(m, old.dataset.as_deref()));
                update(ctx, |x| {
                    if auto {
                        if let Some(m) = &x.model {
                            x.name = state::suggest_name(m, Some(&p));
                        }
                    }
                    x.dataset = Some(p);
                });
                if auto {
                    name_rev.set(name_rev.get_untracked() + 1);
                }
            }
        }
        let items: Vec<(String, String)> = list.iter().map(|d| (d.path.clone(), tr!("training.form.dataset_item", name = d.name.clone(), n = d.rows))).collect();
        vec![Box::new(choice(320.0, items, cur.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(), move |v: &str| {
            let p = std::path::PathBuf::from(v);
            let old = ctx.form.get_untracked();
            let auto = old.name.is_empty() || old.model.as_ref().is_some_and(|m| old.name == state::suggest_name(m, old.dataset.as_deref()));
            update(ctx, |x| {
                if auto {
                    if let Some(m) = &x.model {
                        x.name = state::suggest_name(m, Some(&p));
                    }
                }
                x.dataset = Some(p.clone());
            });
            if auto {
                name_rev.set(name_rev.get_untracked() + 1);
            }
        }))]
    });
    let name = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        name_rev.get();
        let cur = ctx.form.get_untracked().name;
        vec![Box::new(
            TextField::with_text(cur)
                .placeholder(tr!("training.form.name.placeholder"))
                .width(320.0)
                .on_change(move |s: &str| {
                    let s = s.to_string();
                    update(ctx, |x| x.name = s);
                }),
        )]
    });

    let rank = choice(
        160.0,
        [8usize, 16, 32, 64].iter().map(|r| (r.to_string(), r.to_string())).collect(),
        f.rank.to_string(),
        move |v: &str| {
            if let Ok(r) = v.parse() {
                update(ctx, |x| x.rank = r);
            }
        },
    );
    let lrs = [("0.00005", "5·10⁻⁵"), ("0.0001", "1·10⁻⁴"), ("0.0002", "2·10⁻⁴"), ("0.0005", "5·10⁻⁴")];
    let cur_lr = lrs.iter().min_by(|a, b| (a.0.parse::<f32>().unwrap() - f.lr).abs().total_cmp(&(b.0.parse::<f32>().unwrap() - f.lr).abs())).map(|x| x.0).unwrap_or("0.0002");
    let lr = choice(160.0, lrs.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect(), cur_lr.to_string(), move |v: &str| {
        if let Ok(r) = v.parse() {
            update(ctx, |x| x.lr = r);
        }
    });
    let epochs = SpinBox::new().range(1.0, 20.0).step(1.0).value(f.epochs as f64).decimal_places(0).width(160.0).on_change(move |v| {
        update(ctx, |x| x.epochs = v.round().max(1.0) as usize);
    });
    let max_len = choice(
        160.0,
        [512usize, 1024, 2048, 4096, 8192].iter().map(|r| (r.to_string(), r.to_string())).collect(),
        f.max_len.to_string(),
        move |v: &str| {
            if let Ok(r) = v.parse() {
                update(ctx, |x| x.max_len = r);
            }
        },
    );
    let quant = choice(
        220.0,
        vec![
            ("nvfp4".into(), tr!("training.form.quant.nvfp4")),
            ("mxfp8".into(), tr!("training.form.quant.mxfp8")),
            ("bf16".into(), tr!("training.form.quant.bf16")),
        ],
        f.quant.clone(),
        move |v: &str| {
            let v = v.to_string();
            update(ctx, |x| x.quant = v);
        },
    );
    let targets = choice(
        220.0,
        vec![("all".into(), tr!("training.form.targets.all")), ("attn".into(), tr!("training.form.targets.attn"))],
        f.targets.clone(),
        move |v: &str| {
            let v = v.to_string();
            update(ctx, |x| x.targets = v);
        },
    );
    let thinking = Toggle::with_state(f.thinking).on_change(move |v| update(ctx, |x| x.thinking = v)).class("training-toggle");
    // Продолжение — из адаптеров выбранной модели.
    let resume = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let model = ctx.form.get().model;
        let loras = ctx.loras.get();
        let cur = ctx.form.get_untracked().resume.unwrap_or_default();
        let mut items = vec![(String::new(), tr!("training.form.resume.none"))];
        items.extend(loras.iter().filter(|l| l.base.is_some() && l.base == model).map(|l| (l.name.clone(), l.name.clone())));
        vec![Box::new(choice(220.0, items, cur, move |v: &str| {
            let v = (!v.is_empty()).then(|| v.to_string());
            update(ctx, |x| x.resume = v);
        }))]
    });

    let main = mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("training-card training-form") => [
            field(tr!("training.form.model"), tr!("training.form.model.hint"), model_dd),
            field(tr!("training.form.dataset"), tr!("training.form.dataset.hint"), ds_dd),
            field(tr!("training.form.name"), tr!("training.form.name.hint"), name),
        ]
    };
    let params = mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("training-card training-form") => [
            field(tr!("training.form.epochs"), tr!("training.form.epochs.hint"), epochs),
            field(tr!("training.form.lr"), tr!("training.form.lr.hint"), lr),
            field(tr!("training.form.rank"), tr!("training.form.rank.hint"), rank),
            field(tr!("training.form.max_len"), tr!("training.form.max_len.hint"), max_len),
            field(tr!("training.form.quant"), tr!("training.form.quant.hint"), quant),
            field(tr!("training.form.targets"), tr!("training.form.targets.hint"), targets),
            field(tr!("training.form.thinking"), tr!("training.form.thinking.hint"), thinking),
            field(tr!("training.form.resume"), tr!("training.form.resume.hint"), resume),
        ]
    };
    let summary = Reactive::new(move || -> Vec<Box<dyn Widget>> { vec![check_card(ctx)] });
    let buttons = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let f = ctx.form.get();
        let ready = f.model.is_some() && f.dataset.is_some() && !f.name.trim().is_empty();
        let running = ctx.check.get() == Check::Running;
        vec![Box::new(mgui! {
            Row::new().gap(10.0) => [
                Button::new(tr!("training.form.check")).icon(MI_CHECK).disabled(!ready || running).on_click(move || state::check_dataset(ctx)).class("training-btn"),
                Button::new(tr!("training.form.start")).icon(MI_PLAY_ARROW).disabled(!ready).on_click(move || state::start_job(ctx)).class("training-btn training-btn-primary"),
            ]
        })]
    });
    let col = mgui! {
        Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            main,
            Text::new(tr!("training.form.params")).class("training-section-title"),
            params,
            summary,
            buttons,
            Text::new(tr!("training.form.footer")).max_lines(6).class("training-hint"),
        ]
    };
    ScrollView::new().vertical().child(DecoratedBox::new().class("training-center-body").child(col))
}

/// Скорость обучения на этой карте по замерам (RTX 5090 Laptop): токенов в
/// секунду обратно пропорционально весу базы в NVFP4 (1.7B, 2 ГБ — 3100;
/// 9B, 7.4 ГБ — 810). У MoE на токен работает малая доля весов, но эксперты
/// подкачиваются из RAM: Qwen3.6-35B-A3B, 22.6 ГБ — около 950.
pub fn tokens_per_sec(disk_bytes: u64, moe: bool) -> f64 {
    let k = if moe { 2.1e13 } else { 6.2e12 };
    k / disk_bytes.max(1) as f64
}

pub fn hms(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        tr!("training.time.hm", h = s / 3600, m = s / 60 % 60)
    } else if s >= 60 {
        tr!("training.time.ms", m = s / 60, s = s % 60)
    } else {
        tr!("training.time.s", s = s)
    }
}

fn check_card(ctx: TrainingCtx) -> Box<dyn Widget> {
    let body: Box<dyn Widget> = match ctx.check.get() {
        Check::None => Box::new(Text::new(tr!("training.check.none")).max_lines(3).class("training-hint")),
        Check::Running => Box::new(mgui! {
            Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
                ProgressBar::new().indeterminate().class("training-check-progress"),
                Text::new(tr!("training.check.running")).class("training-hint"),
            ]
        }),
        Check::Failed(e) => Box::new(Text::new(e).max_lines(4).class("training-error-text")),
        Check::Done(c) => {
            let f = ctx.form.get_untracked();
            let reg = use_context::<SynModelRegistry>();
            let (disk, moe) = reg.models.get_untracked().into_iter().find(|m| Some(&m.id) == f.model.as_ref()).map(|m| (m.disk_bytes, m.moe())).unwrap_or((0, false));
            let eta = (c.tokens as f64 * f.epochs as f64) / tokens_per_sec(disk, moe);
            let mut col = Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Text::new(tr!("training.check.samples", n = c.samples, tokens = c.tokens, target = c.target_tokens)).max_lines(3).class("training-card-title"))
                .child(Text::new(tr!("training.check.eta", time = hms(eta))).class("training-hint"));
            if c.skipped > 0 || c.truncated > 0 {
                col = col.child(Text::new(tr!("training.check.issues", skipped = c.skipped, truncated = c.truncated)).max_lines(3).class("training-warning"));
            }
            if c.last_turn_only > 0 {
                col = col.child(Text::new(tr!("training.check.last_turn", n = c.last_turn_only)).max_lines(3).class("training-hint"));
            }
            Box::new(col)
        }
    };
    Box::new(DecoratedBox::new().class("training-card").child(body))
}
