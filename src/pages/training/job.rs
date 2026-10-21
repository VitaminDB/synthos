//! Ход задачи обучения: состояние, прогресс, кривая лосса, итог.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widget::styled::StyledWidget;
use syngui::widgets::charts::{AxisConfig, LegendPosition, LineChart, Series};
use synaptixd::{TrainJob, TrainState};

use crate::icons::*;

use super::form::hms;
use super::left::state_text;
use super::state::{self, Selection, TrainingCtx};

pub fn header_actions(ctx: TrainingCtx, id: u64) -> impl Widget {
    let job = ctx.job(id);
    let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if let Some(j) = job {
        if !j.state.finished() {
            row = row.child(
                Button::new(tr!("training.job.stop"))
                    .icon(MI_STOP)
                    .on_click(move || state::act(ctx, move |c| c.train_stop(id), |_| ()))
                    .class("training-btn training-btn-danger"),
            );
        } else {
            if j.state == TrainState::Done || std::path::Path::new(&j.out).join("adapter_model.safetensors").is_file() {
                let name = j.name.clone();
                row = row.child(Button::new(tr!("training.job.open_lora")).icon(MI_LAYERS).on_click(move || ctx.selected.set(Selection::Lora(name.clone()))).class("training-btn training-btn-primary"));
            }
            let again = j.clone();
            row = row.child(
                ToolButton::new(MI_HISTORY)
                    .tooltip(tr!("training.job.again"))
                    .on_click(move || {
                        let mut f = ctx.form.get_untracked();
                        f.model = Some(again.model.clone());
                        f.dataset = Some(std::path::PathBuf::from(&again.dataset));
                        f.name = again.name.clone();
                        let o = &again.options;
                        f.rank = o.lora_r.unwrap_or(f.rank);
                        f.lr = o.lr.unwrap_or(f.lr);
                        f.epochs = o.epochs.unwrap_or(f.epochs);
                        f.max_len = o.max_len.unwrap_or(f.max_len);
                        f.quant = o.quant.clone().unwrap_or(f.quant);
                        ctx.form.set(f);
                        ctx.check.set(state::Check::None);
                        ctx.selected.set(Selection::NewJob);
                    })
                    .class("training-icon-btn"),
            );
            row = row.child(
                ToolButton::new(MI_DELETE)
                    .tooltip(tr!("training.job.forget"))
                    .on_click(move || state::act(ctx, move |c| c.train_forget(id), move |_| ctx.selected.set(Selection::Overview)))
                    .class("training-icon-btn"),
            );
        }
    }
    row
}

fn progress_bar(p: f32, busy: bool) -> StyledWidget<ProgressBar> {
    let b = ProgressBar::new().value(p);
    if busy { b.indeterminate() } else { b }.class("training-progress")
}

fn stat(label: String, value: String) -> Box<dyn Widget> {
    Box::new(mgui! {
        Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start).class("training-stat") => [
            Text::new(value).class("training-stat-value"),
            Text::new(label).class("training-stat-label"),
        ]
    })
}

fn progress_line(j: &TrainJob) -> String {
    match j.state {
        TrainState::Running if j.total_steps > 0 => tr!(
            "training.job.progress",
            step = j.step,
            total = j.total_steps,
            pct = (100 * j.step / j.total_steps.max(1)),
            eta = hms(j.eta_secs as f64)
        ),
        TrainState::Preparing => tr!("training.job.preparing"),
        TrainState::Queued => tr!("training.job.queued"),
        _ => state_text(j),
    }
}

/// Карточка задачи для обзора.
pub fn summary_card(ctx: TrainingCtx, j: &TrainJob) -> Box<dyn Widget> {
    let id = j.id;
    let p = if j.total_steps > 0 { j.step as f32 / j.total_steps as f32 } else { 0.0 };
    Box::new(
        syngui::widgets::containers::GestureDetector::new().on_click(move || ctx.selected.set(Selection::Job(id))).child(
            DecoratedBox::new().class("training-card").child(mgui! {
                Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    Text::new(tr!("training.job.summary", name = j.name.clone(), model = j.model_title.clone())).class("training-card-title"),
                    progress_bar(p, j.state == TrainState::Preparing),
                    Text::new(progress_line(j)).class("training-hint"),
                ]
            }),
        ),
    )
}

fn chart(j: &TrainJob) -> Box<dyn Widget> {
    if j.history.len() < 2 {
        return Box::new(Text::new(tr!("training.job.chart.empty")).class("training-hint"));
    }
    let train: Vec<(f64, f64)> = j.history.iter().map(|p| (p.step as f64, p.loss as f64)).collect();
    let eval: Vec<(f64, f64)> = j.history.iter().filter_map(|p| p.eval_loss.map(|e| (p.step as f64, e as f64))).collect();
    let max = train.iter().chain(eval.iter()).map(|p| p.1).fold(0.0, f64::max);
    let min = train.iter().chain(eval.iter()).map(|p| p.1).fold(f64::MAX, f64::min);
    let pad = ((max - min) * 0.1).max(0.02);
    let mut c = LineChart::new()
        .x_axis(AxisConfig::new().min(1.0).max(j.total_steps.max(j.step).max(2) as f64).tick_count(5).format(|v| format!("{v:.0}")).axis_line(false))
        .y_axis(AxisConfig::new().min((min - pad).max(0.0)).max(max + pad).tick_count(4).format(|v| format!("{v:.2}")).axis_line(false))
        .legend(LegendPosition::Top)
        .tooltip(true)
        .animate(false)
        .height(260.0)
        .series(Series::new(tr!("training.job.chart.train")).data(train).line_width(2.0).area_fill(0.12).show_points(false).smooth(true));
    if !eval.is_empty() {
        c = c.series(Series::new(tr!("training.job.chart.eval")).data(eval).line_width(2.0).show_points(true).point_size(5.0));
    }
    Box::new(c.class("training-chart"))
}

pub fn view(ctx: TrainingCtx, id: u64) -> impl Widget {
    let body = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(j) = ctx.job(id) else {
            return vec![Box::new(Text::new(tr!("training.job.missing")).class("training-hint"))];
        };
        let p = if j.total_steps > 0 { j.step as f32 / j.total_steps as f32 } else { 0.0 };
        let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        let status = mgui! {
            Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("training-card") => [
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(state_text(&j)).class(match j.state {
                    TrainState::Failed => "training-badge training-badge-error",
                    TrainState::Done => "training-badge training-badge-ok",
                    TrainState::Running | TrainState::Preparing => "training-badge training-badge-run",
                    _ => "training-badge",
                })).child(Text::new(if j.state.finished() {
                    format!("{}: {} / {} · {}", tr!("training.lora.steps"), j.step, j.total_steps, hms(j.elapsed_secs as f64))
                } else {
                    progress_line(&j)
                }).max_lines(2).class("training-hint")),
                progress_bar(p, j.state == TrainState::Preparing),
            ]
        };
        col = col.child(status);
        if let Some(e) = &j.error {
            col = col.child(DecoratedBox::new().class("training-card").child(Text::new(e.clone()).max_lines(8).class("training-error-text")));
        }
        if j.step > 0 {
            let stats = Row::new()
                .gap(28.0)
                .child(stat(tr!("training.job.stat.loss"), format!("{:.4}", j.loss)))
                .child(stat(tr!("training.job.stat.eval"), j.eval_loss.map(|e| format!("{e:.4}")).unwrap_or_else(|| "—".into())))
                .child(stat(tr!("training.job.stat.speed"), tr!("training.job.stat.speed_value", v = format!("{:.0}", j.tokens_per_sec))))
                .child(stat(tr!("training.job.stat.elapsed"), hms(j.elapsed_secs as f64)))
                .child(stat(tr!("training.job.stat.epoch"), format!("{}", j.epoch + 1)))
                .child(stat(tr!("training.job.stat.lr"), format!("{:.1e}", j.lr)));
            col = col.child(DecoratedBox::new().class("training-card").child(stats));
        }
        col = col.child(DecoratedBox::new().class("training-card").child(mgui! {
            Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                Text::new(tr!("training.job.chart.title")).class("training-card-title"),
                chart(&j),
            ]
        }));
        let mut facts = Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new(tr!("training.job.facts.title")).class("training-card-title"))
            .child(Text::new(tr!("training.job.facts.model", v = j.model_title.clone())).max_lines(2).class("training-hint"))
            .child(Text::new(tr!("training.job.facts.dataset", v = crate::paths::pretty(std::path::Path::new(&j.dataset)))).max_lines(2).class("training-hint"))
            .child(Text::new(tr!("training.job.facts.out", v = crate::paths::pretty(std::path::Path::new(&j.out)))).max_lines(2).class("training-hint"));
        if let Some(c) = &j.dataset_check {
            facts = facts.child(Text::new(tr!("training.check.samples", n = c.samples, tokens = c.tokens, target = c.target_tokens)).max_lines(2).class("training-hint"));
        }
        let o = &j.options;
        facts = facts.child(
            Text::new(tr!(
                "training.job.facts.params",
                rank = o.lora_r.unwrap_or(16),
                lr = format!("{:.0e}", o.lr.unwrap_or(2e-4)),
                epochs = o.epochs.unwrap_or(1),
                len = o.max_len.unwrap_or(2048),
                quant = o.quant.clone().unwrap_or_else(|| "nvfp4".into()).to_uppercase()
            ))
            .max_lines(3)
            .class("training-hint"),
        );
        col = col.child(DecoratedBox::new().class("training-card").child(facts));
        if !j.state.finished() {
            col = col.child(Text::new(tr!("training.job.background")).max_lines(4).class("training-hint"));
        }
        vec![Box::new(col)]
    });
    ScrollView::new().vertical().child(DecoratedBox::new().class("training-center-body").child(body))
}
