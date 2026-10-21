//! Левая панель «Обучения»: задачи, датасеты и адаптеры библиотек.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use synaptixd::{TrainJob, TrainState};

use crate::components::panel_header;
use crate::icons::*;

use super::state::{Selection, TrainingCtx};

pub fn header() -> impl Widget {
    panel_header::side_title(MI_MODEL_TRAINING, tr!("training.title"))
}

pub fn view() -> impl Widget {
    let ctx = use_context::<TrainingCtx>();
    DecoratedBox::new().class("training-left").child(ScrollView::new().vertical().child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let sel = ctx.selected.get();
        let jobs = ctx.jobs.get();
        let datasets = ctx.datasets.get();
        let loras = ctx.loras.get();
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);

        col = col.child(item(ctx, MI_INFO, tr!("training.overview.nav"), String::new(), sel == Selection::Overview, Selection::Overview, None));

        col = col.child(section(tr!("training.left.jobs"), Some((tr!("training.new.button"), Box::new(move || super::form::open_new(ctx))))));
        if jobs.is_empty() {
            col = col.child(Text::new(tr!("training.left.jobs.empty")).class("training-left-empty"));
        }
        for j in &jobs {
            col = col.child(item(ctx, job_icon(j), j.name.clone(), job_line(j), sel == Selection::Job(j.id), Selection::Job(j.id), job_progress(j)));
        }

        col = col.child(section(tr!("training.left.datasets"), Some((tr!("training.dataset.new"), Box::new(move || ask_new_dataset(ctx))))));
        if datasets.is_empty() {
            col = col.child(Text::new(tr!("training.left.datasets.empty")).max_lines(4).class("training-left-empty"));
        }
        for d in &datasets {
            let path = std::path::PathBuf::from(&d.path);
            let on = sel == Selection::Dataset(path.clone());
            let p = path.clone();
            col = col.child(clickable(
                row_body(MI_DATASET, d.name.clone(), tr!("training.left.dataset.line", n = d.rows, size = mb(d.bytes)), on, None),
                move || super::dataset::open(ctx, p.clone()),
            ));
        }

        col = col.child(section(tr!("training.left.loras"), None));
        if loras.is_empty() {
            col = col.child(Text::new(tr!("training.left.loras.empty")).max_lines(4).class("training-left-empty"));
        }
        for l in &loras {
            let base = l.train.get("base_title").and_then(|v| v.as_str()).map(String::from).or_else(|| l.base.clone()).unwrap_or_default();
            let line = match l.train.get("eval_loss").and_then(|v| v.as_f64()) {
                Some(e) => tr!("training.left.lora.line_eval", base = base, eval = format!("{e:.3}")),
                None => base,
            };
            col = col.child(item(ctx, MI_LAYERS, l.name.clone(), line, sel == Selection::Lora(l.name.clone()), Selection::Lora(l.name.clone()), None));
        }
        vec![Box::new(DecoratedBox::new().class("training-left-body").child(col))]
    })))
}

fn mb(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.1} ГБ", b as f64 / (1u64 << 30) as f64)
    } else {
        format!("{} МБ", (b >> 20).max(1))
    }
}

fn job_icon(j: &TrainJob) -> &'static str {
    match j.state {
        TrainState::Queued => MI_HOURGLASS_TOP,
        TrainState::Preparing | TrainState::Running => MI_MODEL_TRAINING,
        TrainState::Done => MI_CHECK,
        TrainState::Failed => MI_REPORT,
        TrainState::Cancelled => MI_STOP,
    }
}

pub fn state_text(j: &TrainJob) -> String {
    match j.state {
        TrainState::Queued => tr!("training.state.queued"),
        TrainState::Preparing => tr!("training.state.preparing"),
        TrainState::Running => tr!("training.state.running"),
        TrainState::Done => tr!("training.state.done"),
        TrainState::Failed => tr!("training.state.failed"),
        TrainState::Cancelled => tr!("training.state.cancelled"),
    }
}

fn job_line(j: &TrainJob) -> String {
    match j.state {
        TrainState::Running if j.total_steps > 0 => tr!("training.left.job.running", step = j.step, total = j.total_steps, loss = format!("{:.3}", j.loss)),
        _ => format!("{} · {}", state_text(j), j.model_title),
    }
}

fn job_progress(j: &TrainJob) -> Option<f32> {
    match j.state {
        TrainState::Running if j.total_steps > 0 => Some(j.step as f32 / j.total_steps as f32),
        TrainState::Preparing => Some(0.0),
        _ => None,
    }
}

fn section(title: String, action: Option<(String, Box<dyn Fn() + Send + Sync>)>) -> Box<dyn Widget> {
    let mut row = Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(title).class("training-left-section"))
        .child(DecoratedBox::new().class("grow"));
    if let Some((tip, f)) = action {
        row = row.child(ToolButton::new(MI_ADD).tooltip(tip).on_click(move || f()).class("training-icon-btn"));
    }
    Box::new(DecoratedBox::new().class("training-left-section-row").child(row))
}

fn row_body(icon: &'static str, title: String, line: String, selected: bool, progress: Option<f32>) -> Box<dyn Widget> {
    let mut info = Column::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(title).max_lines(1).class("training-item-title"));
    if !line.is_empty() {
        info = info.child(Text::new(line).max_lines(1).class("training-item-line"));
    }
    if let Some(p) = progress {
        info = info.child(ProgressBar::new().value(p).class("training-item-progress"));
    }
    Box::new(DecoratedBox::new().class(if selected { "training-item training-item-selected" } else { "training-item" }).child(mgui! {
        Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Icon::new(icon).class("training-item-icon"),
            DecoratedBox::new().class("grow").child(info),
        ]
    }))
}

fn clickable(body: Box<dyn Widget>, f: impl Fn() + Send + Sync + 'static) -> Box<dyn Widget> {
    Box::new(GestureDetector::new().on_click(move || f()).child(Stack::new().children(vec![body])))
}

fn item(ctx: TrainingCtx, icon: &'static str, title: String, line: String, selected: bool, to: Selection, progress: Option<f32>) -> Box<dyn Widget> {
    clickable(row_body(icon, title, line, selected, progress), move || ctx.selected.set(to.clone()))
}

/// Спросить имя нового датасета и создать его.
pub fn ask_new_dataset(ctx: TrainingCtx) {
    let n = ctx.datasets.get_untracked().len() + 1;
    super::dataset::create(ctx, &tr!("training.dataset.default_name", n = n));
}
