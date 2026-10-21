//! Снимок страницы «Обучение» (или карточки адаптеров чата) без окна — для
//! проверки вёрстки с живыми данными службы нейросетей.
//!
//! `cargo run --example training_shot -- out.png [overview|new|job:<id>|dataset:<путь>|lora:<имя>|chat] [ширина]x[высота]`

use syngui::prelude::*;
use synthos::pages::training::state::Selection;
use synthos::pages::training::TrainingCtx;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().cloned().unwrap_or_else(|| "training.png".into());
    let what = args.get(1).cloned().unwrap_or_else(|| "overview".into());
    let (w, h) = args
        .get(2)
        .and_then(|s| s.split_once('x'))
        .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
        .unwrap_or((1280u32, 860u32));
    syngui::signal::init_main_thread();
    syngui::signal::allow_signal_reads_on_this_thread();
    syngui::i18n::register_catalogs(&synthos::i18n::CATALOGS);
    syngui::i18n::set_language(syngui::i18n::Lang::new(&std::env::var("SHOT_LANG").unwrap_or_else(|_| "ru".into())));
    let (theme, ctx) = synthos::build_context();
    provide_context(ctx);
    let cfg = synthos::config::AppConfig::load();
    provide_context(synthos::pages::node_editor::tabs::EditorWorkspace::new_or_restore());
    provide_context(synthos::pages::syn_explorer::SynExplorerCtx::new(&cfg));
    provide_context(synthos::syn_chat::SynChatCtx::new());
    provide_context(synthos::pages::notes::NotesCtx::new_or_restore(&synthos::notes_host::state(&cfg)));
    provide_context(synthos::pages::huggingface::HuggingFaceCtx::new(&cfg));
    provide_context(synthos::syn_chat::SynModelRegistry::new());
    let training = TrainingCtx::new();
    provide_context(training);
    synthos::search::install();
    let mss = format!("{}\n{}", synthos::styles::styles(), theme.get_untracked());
    let sel = match what.split_once(':') {
        Some(("job", id)) => Selection::Job(id.parse().unwrap_or(1)),
        Some(("dataset", p)) => Selection::Dataset(p.into()),
        Some(("lora", n)) => Selection::Lora(n.into()),
        _ if what == "new" => Selection::NewJob,
        _ => Selection::Overview,
    };
    let chat = what == "chat";
    let opts = syngui::shot::ShotOptions {
        ready: Some(Box::new(move || training.online.get_untracked() && !training.loras.get_untracked().is_empty())),
        settle: std::time::Duration::from_millis(1500),
        timeout: std::time::Duration::from_secs(20),
        ..Default::default()
    };
    let r = syngui::shot::render_png_with(
        move || {
            if let Selection::Dataset(p) = &sel {
                synthos::pages::training::dataset::open(training, p.clone());
            } else if sel == Selection::NewJob {
                synthos::pages::training::form::open_new(training);
            } else {
                training.selected.set(sel.clone());
            }
            if chat {
                Box::new(synthos::pages::syn_chat::right_panel::body())
            } else {
                Box::new(synthos::pages::training::view())
            }
        },
        &mss,
        (w, h),
        1.0,
        &out,
        opts,
    );
    match r {
        Ok((pw, ph)) => println!("{out}: {pw}x{ph}"),
        Err(e) => eprintln!("снимок: {e}"),
    }
}
