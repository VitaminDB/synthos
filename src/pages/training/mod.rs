//! Страница «Обучение» — дообучение языковых моделей адаптерами LoRA
//! через службу нейросетей.
//!
//! ```text
//! workspace_frame [
//!   left: задачи (идут, в очереди, готовые) · датасеты · адаптеры
//!   center: обзор | форма новой задачи | ход задачи (график лосса) |
//!           редактор датасета | адаптер
//! ]
//! ```
//!
//! Обучение идёт в службе (`synaptixd`), Synthos только ставит задачу и
//! смотрит ход: окно можно закрыть, задача продолжится. Готовый адаптер
//! включается в «Параметрах» модели или в карточке чата.

pub mod dataset;
pub mod form;
pub mod job;
pub mod left;
pub mod lora;
pub mod state;

use syngui::prelude::*;

pub use state::TrainingCtx;

use crate::components::panel_header::{self, CenterSpec};
use crate::components::workspace_frame::{self, expand, FrameSpec, Pane};
use crate::icons::*;
use state::Selection;

pub fn view() -> impl Widget {
    let ctx = use_context::<TrainingCtx>();
    state::refresh(ctx);
    let spec = FrameSpec::new(
        "training-h-split",
        move || {
            let identity: Box<dyn Widget> = Box::new(DecoratedBox::new().child(move || header_identity(ctx)));
            let actions = DecoratedBox::new().child(move || header_actions(ctx));
            Box::new(panel_header::center(CenterSpec::new(identity).actions(actions)))
        },
        move || Box::new(DecoratedBox::new().class("training-center").child(move || center(ctx))),
    )
    .left(Pane::new(ctx.left_visible, ctx.left_ratio, 240.0, || Box::new(left::header()), || Box::new(left::view())));
    DecoratedBox::new().class("training-page").child(workspace_frame::view(spec))
}

fn header_identity(ctx: TrainingCtx) -> Stack {
    let (icon, title, sub) = match ctx.selected.get() {
        Selection::Overview => (MI_MODEL_TRAINING, tr!("training.title"), tr!("training.subtitle")),
        Selection::NewJob => (MI_ADD, tr!("training.new.title"), tr!("training.new.subtitle")),
        Selection::Job(id) => match ctx.job(id) {
            Some(j) => (MI_MODEL_TRAINING, j.name.clone(), tr!("training.job.subtitle", model = j.model_title.clone())),
            None => (MI_MODEL_TRAINING, tr!("training.title"), String::new()),
        },
        Selection::Dataset(p) => {
            let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let n = ctx.doc.get().map(|d| {
                d.rev.get();
                d.len()
            });
            (MI_DATASET, name, n.map(|n| tr!("training.dataset.subtitle", n = n)).unwrap_or_default())
        }
        Selection::Lora(n) => (MI_LAYERS, n, tr!("training.lora.subtitle")),
    };
    Stack::new().children(vec![panel_header::identity_text(icon, title, sub)])
}

fn header_actions(ctx: TrainingCtx) -> Stack {
    let w: Box<dyn Widget> = match ctx.selected.get() {
        Selection::Dataset(_) => match ctx.doc.get() {
            Some(d) => Box::new(dataset::header_actions(ctx, d)),
            None => Box::new(DecoratedBox::new()),
        },
        Selection::Job(id) => Box::new(job::header_actions(ctx, id)),
        Selection::NewJob => Box::new(DecoratedBox::new()),
        _ => Box::new(
            Button::new(tr!("training.new.button"))
                .icon(MI_ADD)
                .on_click(move || form::open_new(ctx))
                .class("training-btn training-btn-primary"),
        ),
    };
    Stack::new().children(vec![w])
}

fn center(ctx: TrainingCtx) -> Stack {
    let error = ctx.error.get();
    let body: Box<dyn Widget> = if !ctx.online.get() {
        Box::new(offline())
    } else {
        match ctx.selected.get() {
            Selection::Overview => Box::new(overview(ctx)),
            Selection::NewJob => Box::new(form::view(ctx)),
            Selection::Job(id) => Box::new(job::view(ctx, id)),
            Selection::Dataset(_) => match ctx.doc.get() {
                Some(d) => Box::new(dataset::view(ctx, d)),
                None => Box::new(overview(ctx)),
            },
            Selection::Lora(n) => Box::new(lora::view(ctx, n)),
        }
    };
    let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if let Some(e) = error {
        col = col.child(
            DecoratedBox::new().class("training-error-bar").child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(MI_REPORT).class("training-error-icon"))
                    .child(DecoratedBox::new().class("grow").child(Text::new(e).max_lines(3).class("training-error-text")))
                    .child(ToolButton::new(MI_CLOSE).on_click(move || ctx.error.set(None)).class("training-icon-btn")),
            ),
        );
    }
    col = col.child(DecoratedBox::new().class("grow").child(expand(body)));
    let w: Box<dyn Widget> = Box::new(col);
    Stack::new().fit(StackFit::Expand).children(vec![w])
}

fn offline() -> impl Widget {
    Center::new().child(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(MI_REPORT).class("training-empty-icon"))
            .child(Text::new(tr!("training.offline.title")).class("training-empty-title"))
            .child(Text::new(tr!("training.offline.hint")).max_lines(4).class("training-hint")),
    )
}

/// Обзор: три шага и текущие задачи.
fn overview(ctx: TrainingCtx) -> impl Widget {
    let step = |n: &str, title: String, text: String| -> Box<dyn Widget> {
        Box::new(
            DecoratedBox::new().class("training-card training-step").child(
                Row::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(DecoratedBox::new().class("training-step-num").child(Center::new().child(Text::new(n.to_string()).class("training-step-num-text"))))
                    .child(
                        DecoratedBox::new().class("grow").child(
                            Column::new()
                                .gap(4.0)
                                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                                .child(Text::new(title).class("training-card-title"))
                                .child(Text::new(text).max_lines(6).class("training-hint")),
                        ),
                    ),
            ),
        )
    };
    let jobs = ctx.jobs.get();
    let running: Vec<_> = jobs.iter().filter(|j| !j.state.finished()).cloned().collect();
    let mut col = Column::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(tr!("training.overview.lead")).max_lines(6).class("training-lead"))
        .child(step("1", tr!("training.overview.step1.title"), tr!("training.overview.step1.text")))
        .child(step("2", tr!("training.overview.step2.title"), tr!("training.overview.step2.text")))
        .child(step("3", tr!("training.overview.step3.title"), tr!("training.overview.step3.text")));
    for j in running {
        col = col.child(job::summary_card(ctx, &j));
    }
    col = col.child(
        Row::new()
            .gap(10.0)
            .child(Button::new(tr!("training.new.button")).icon(MI_ADD).on_click(move || form::open_new(ctx)).class("training-btn training-btn-primary"))
            .child(Button::new(tr!("training.dataset.new")).icon(MI_DATASET).on_click(move || left::ask_new_dataset(ctx)).class("training-btn")),
    );
    ScrollView::new().vertical().child(DecoratedBox::new().class("training-center-body").child(col))
}
