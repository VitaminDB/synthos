//! Состояние страницы «Обучение» и связь со службой нейросетей.
//!
//! Обучение идёт в службе (`synaptixd`): задача живёт там и после закрытия
//! Synthos, ход приходит событиями подписки. Здесь — зеркало: задачи,
//! датасеты и адаптеры библиотек, выбор в левой панели, форма новой задачи
//! и редактор датасета. Все запросы к службе — в фоновых потоках, ответ
//! возвращается на главный поток через `run_on_main_thread`.
//!
//! Тот же контекст отдаёт список адаптеров чату (карточка «Адаптеры LoRA»).

use std::path::PathBuf;

use syngui::prelude::*;
use synaptixd::{DatasetCheck, DatasetEntry, LoraEntry, TrainJob, TrainOptions};

use super::dataset::DatasetDoc;

/// Что открыто в центре страницы.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Selection {
    /// Обзор: как это устроено и что можно сделать.
    #[default]
    Overview,
    /// Форма новой задачи.
    NewJob,
    Job(u64),
    Dataset(PathBuf),
    Lora(String),
}

/// Форма новой задачи обучения.
#[derive(Clone, Debug, PartialEq)]
pub struct JobForm {
    pub model: Option<String>,
    pub dataset: Option<PathBuf>,
    pub name: String,
    pub rank: usize,
    pub lr: f32,
    pub epochs: usize,
    pub max_len: usize,
    pub grad_accum: usize,
    pub quant: String,
    pub targets: String,
    pub thinking: bool,
    pub resume: Option<String>,
}

impl Default for JobForm {
    fn default() -> Self {
        Self {
            model: None,
            dataset: None,
            name: String::new(),
            rank: 16,
            lr: 2e-4,
            epochs: 1,
            max_len: 2048,
            grad_accum: 4,
            quant: "nvfp4".into(),
            targets: "all".into(),
            thinking: false,
            resume: None,
        }
    }
}

impl JobForm {
    pub fn options(&self) -> TrainOptions {
        TrainOptions {
            lora_r: Some(self.rank),
            lr: Some(self.lr),
            epochs: Some(self.epochs),
            max_len: Some(self.max_len),
            grad_accum: Some(self.grad_accum),
            quant: Some(self.quant.clone()),
            targets: Some(self.targets.clone()),
            thinking: self.thinking,
            resume: self.resume.clone(),
            ..Default::default()
        }
    }
}

/// Проверка датасета токенизатором модели.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Check {
    #[default]
    None,
    Running,
    Done(DatasetCheck),
    Failed(String),
}

#[derive(Clone, Copy)]
pub struct TrainingCtx {
    pub online: RwSignal<bool>,
    pub jobs: RwSignal<Vec<TrainJob>>,
    pub datasets: RwSignal<Vec<DatasetEntry>>,
    pub loras: RwSignal<Vec<LoraEntry>>,
    /// Корни библиотек службы (куда класть новый датасет).
    pub libraries: RwSignal<Vec<PathBuf>>,
    pub selected: RwSignal<Selection>,
    pub form: RwSignal<JobForm>,
    pub check: RwSignal<Check>,
    /// Открытый в редакторе датасет.
    pub doc: RwSignal<Option<DatasetDoc>>,
    pub left_visible: RwSignal<bool>,
    pub left_ratio: RwSignal<f32>,
    /// Ошибка последнего действия (показывается полосой над центром).
    pub error: RwSignal<Option<String>>,
}

impl TrainingCtx {
    pub fn new() -> Self {
        let ctx = Self {
            online: use_signal(false),
            jobs: use_signal(Vec::new()),
            datasets: use_signal(Vec::new()),
            loras: use_signal(Vec::new()),
            libraries: use_signal(Vec::new()),
            selected: use_signal(Selection::Overview),
            form: use_signal(JobForm::default()),
            check: use_signal(Check::None),
            doc: use_signal(None),
            left_visible: use_signal(true),
            left_ratio: use_signal(0.24),
            error: use_signal(None),
        };
        watch(ctx);
        ctx
    }

    pub fn job(&self, id: u64) -> Option<TrainJob> {
        self.jobs.get().into_iter().find(|j| j.id == id)
    }

    /// Задачи, которые идут или ждут.
    pub fn active_jobs(&self) -> usize {
        self.jobs.get().iter().filter(|j| !j.state.finished()).count()
    }
}

impl Default for TrainingCtx {
    fn default() -> Self {
        Self::new()
    }
}

fn set_changed<T: Clone + PartialEq + Send + Sync + 'static>(sig: RwSignal<T>, v: T) {
    if sig.get_untracked() != v {
        sig.set(v);
    }
}

/// Перечитать задачи, датасеты, адаптеры и библиотеки.
pub fn refresh(ctx: TrainingCtx) {
    std::thread::spawn(move || {
        let r = fetch();
        syngui::async_runtime::run_on_main_thread(move || apply(ctx, r));
    });
}

type Snapshot = synaptixd::Result<(Vec<TrainJob>, Vec<DatasetEntry>, Vec<LoraEntry>, Vec<PathBuf>)>;

fn fetch() -> Snapshot {
    let mut c = synaptixd::Client::connect()?;
    let jobs = c.train_jobs()?;
    let datasets = c.datasets()?;
    let loras = c.loras()?;
    let libs = c.libraries().unwrap_or_default().into_iter().filter(|l| l.online).map(|l| PathBuf::from(l.path)).collect();
    Ok((jobs, datasets, loras, libs))
}

fn apply(ctx: TrainingCtx, r: Snapshot) {
    match r {
        Ok((mut jobs, datasets, loras, libs)) => {
            jobs.sort_by(|a, b| b.id.cmp(&a.id));
            set_changed(ctx.online, true);
            set_changed(ctx.jobs, jobs);
            set_changed(ctx.datasets, datasets);
            set_changed(ctx.loras, loras);
            set_changed(ctx.libraries, libs);
        }
        Err(_) => set_changed(ctx.online, false),
    }
}

/// Подписка на службу: ход задач обновляется по событию, списки — по
/// `LorasChanged`; без службы — повтор раз в 5 с.
fn watch(ctx: TrainingCtx) {
    std::thread::Builder::new()
        .name("synaptixd-train".into())
        .spawn(move || loop {
            let r = fetch();
            let ok = r.is_ok();
            syngui::async_runtime::run_on_main_thread(move || apply(ctx, r));
            if ok {
                let sub = synaptixd::Client::connect().and_then(|c| {
                    c.subscribe(|ev| {
                        match ev {
                            synaptixd::Event::Train { job } => {
                                syngui::async_runtime::run_on_main_thread(move || {
                                    let mut jobs = ctx.jobs.get_untracked();
                                    match jobs.iter_mut().find(|j| j.id == job.id) {
                                        Some(j) => *j = job,
                                        None => {
                                            jobs.push(job);
                                            jobs.sort_by(|a, b| b.id.cmp(&a.id));
                                        }
                                    }
                                    ctx.jobs.set(jobs);
                                });
                            }
                            synaptixd::Event::LorasChanged | synaptixd::Event::ModelsChanged => {
                                let r = fetch();
                                syngui::async_runtime::run_on_main_thread(move || apply(ctx, r));
                            }
                            _ => {}
                        }
                        true
                    })
                });
                if let Err(e) = sub {
                    log::warn!("[training] подписка на службу нейросетей: {e}");
                }
            }
            syngui::async_runtime::run_on_main_thread(move || set_changed(ctx.online, false));
            std::thread::sleep(std::time::Duration::from_secs(5));
        })
        .ok();
}

/// Запрос к службе в фоне; ошибка — в полосу ошибки страницы, затем
/// списки перечитываются.
pub fn act<T: Send + 'static>(
    ctx: TrainingCtx,
    f: impl FnOnce(&mut synaptixd::Client) -> synaptixd::Result<T> + Send + 'static,
    done: impl FnOnce(T) + Send + 'static,
) {
    std::thread::spawn(move || {
        let r = synaptixd::Client::connect().and_then(|mut c| f(&mut c));
        let snap = fetch();
        syngui::async_runtime::run_on_main_thread(move || {
            match r {
                Ok(v) => {
                    ctx.error.set(None);
                    done(v);
                }
                Err(e) => ctx.error.set(Some(e.to_string())),
            }
            apply(ctx, snap);
        });
    });
}

/// Запустить проверку датасета для формы.
pub fn check_dataset(ctx: TrainingCtx) {
    let form = ctx.form.get_untracked();
    let (Some(model), Some(path)) = (form.model.clone(), form.dataset.clone()) else { return };
    ctx.check.set(Check::Running);
    let opts = form.options();
    std::thread::spawn(move || {
        let r = synaptixd::Client::connect().and_then(|mut c| c.dataset_check(&model, &path.to_string_lossy(), opts));
        syngui::async_runtime::run_on_main_thread(move || {
            ctx.check.set(match r {
                Ok(c) => Check::Done(c),
                Err(e) => Check::Failed(e.to_string()),
            });
        });
    });
}

/// Поставить задачу из формы в очередь и открыть её.
pub fn start_job(ctx: TrainingCtx) {
    let form = ctx.form.get_untracked();
    let (Some(model), Some(path)) = (form.model.clone(), form.dataset.clone()) else { return };
    let name = form.name.trim().to_string();
    let opts = form.options();
    act(
        ctx,
        move |c| c.train_start(&model, &path.to_string_lossy(), &name, opts),
        move |job| ctx.selected.set(Selection::Job(job.id)),
    );
}

/// Имя адаптера по умолчанию: датасет + модель.
pub fn suggest_name(model: &str, dataset: Option<&std::path::Path>) -> String {
    let ds = dataset.and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let base: String = model.chars().take(24).collect();
    let raw = if ds.is_empty() { base } else { format!("{ds}-{base}") };
    raw.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '.' { c } else { '-' }).collect::<String>().trim_matches('-').to_lowercase()
}
