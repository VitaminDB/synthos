//! Headless smoke-раннер LLM-ноды (synaptix Qwen3 / Hybrid) без GUI: собирает
//! граф «TextView(prompt) → LLM → TextView(answer)», прогоняет worker генерации
//! и печатает ответ. Доказывает полный путь ноды (load → чат-шаблон → encode →
//! generate_streaming → decode → output порт) на реальной модели.
//!
//! Кросс-поточные `RwSignal::set` воркера применяются дренажём
//! `drain_main_thread_callbacks` (без окна канал main-thread колбэков никто не
//! выкачивает).
//!
//! Запуск: `llm_smoke <model_dir|.syn> <prompt> [device:cuda|cpu] [max_tokens] [system]`
//!   device по умолчанию cpu (smoke без GPU); cuda — для GPU-проверки.

use std::time::{Duration, Instant};

use synthos::pages::node_editor::eval;
use synthos::pages::node_editor::nodes::llm;
use synthos::pages::node_editor::state::NodeEditorCtx;
use synthos::pages::node_editor::types::{Connection, NodeId, NodeInstance, NodeKind, NodeRuntime};

fn drain() {
    syngui::async_runtime::drain_main_thread_callbacks();
}

fn refresh(ctx: &NodeEditorCtx) {
    let nodes = ctx.nodes.get_untracked();
    let conns = ctx.connections.get_untracked();
    let map = eval::evaluate_graph(&nodes, &conns, false);
    ctx.values.set(map);
}

fn node(ctx: &NodeEditorCtx, id: NodeId) -> NodeInstance {
    ctx.nodes
        .get_untracked()
        .iter()
        .find(|n| n.id == id)
        .cloned()
        .expect("нода найдена")
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        return Err(format!(
            "использование: {} <model_dir|.syn> <prompt> [device:cuda|cpu] [max_tokens] [system]",
            args[0]
        ));
    }
    let model = std::path::PathBuf::from(&args[1]);
    let prompt = args[2].clone();
    let device = args.get(3).map(|s| s.as_str()).unwrap_or("cpu");
    let max_tokens: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(64);
    let system = args.get(5).cloned().unwrap_or_default();
    if !model.exists() {
        return Err(format!("нет модели: {}", model.display()));
    }
    eprintln!(
        "llm_smoke: model={} device={device} max_tokens={max_tokens}",
        model.display()
    );

    let ctx = NodeEditorCtx::new();
    let zero = syngui::core::Point::new(0.0, 0.0);
    let n_prompt = ctx.add_node(NodeKind::TextView, zero);
    let n_llm = ctx.add_node(NodeKind::Llm, zero);
    let n_ckpt = ctx.add_node(NodeKind::SynCheckpoint, zero);
    let n_answer = ctx.add_node(NodeKind::TextView, zero);

    // Источник вопроса: TextView с заранее выставленным output_text.
    if let NodeRuntime::TextView { output_text, .. } = &*node(&ctx, n_prompt).runtime.lock().unwrap() {
        output_text.set(prompt.clone());
    }

    // Модель — через Syn Checkpoint на входе `model` (своих полей у LLM нет).
    // Предпочтения чекпойнта: device 1=CUDA,2=CPU; storage 3=FP8,4=NVFP4;
    // compute 2=BF16 (для dense; квант форсит F16 внутри).
    if let NodeRuntime::SynCheckpoint {
        model_path,
        device_idx: dev,
        storage_idx,
        compute_idx,
        ..
    } = &*node(&ctx, n_ckpt).runtime.lock().unwrap()
    {
        model_path.set(Some(model.clone()));
        dev.set(if device.eq_ignore_ascii_case("cuda") { 1 } else { 2 });
        // SYN_SMOKE_LLM_QUANT=none|nvfp4|mxfp8 (дефолт none).
        storage_idx.set(match std::env::var("SYN_SMOKE_LLM_QUANT").as_deref() {
            Ok("nvfp4") => 4,
            Ok("mxfp8") => 3,
            _ => 0,
        });
        compute_idx.set(2);
    }

    // Параметры LLM-ноды.
    if let NodeRuntime::Llm {
        system_prompt,
        max_tokens: mt,
        temperature,
        ..
    } = &*node(&ctx, n_llm).runtime.lock().unwrap()
    {
        system_prompt.set(system.clone());
        mt.set(max_tokens);
        temperature.set(0.0); // greedy для детерминированного smoke
    }

    let conn = |from: NodeId, fp: &'static str, to: NodeId, tp: &'static str| Connection {
        from_node: from,
        from_port: fp,
        to_node: to,
        to_port: tp,
    };
    ctx.connections.set(vec![
        conn(n_prompt, "out", n_llm, "prompt"),
        conn(n_ckpt, "model", n_llm, "model"),
        conn(n_llm, "answer", n_answer, "in"),
    ]);

    refresh(&ctx);

    // Снимок running/error до запуска воркера.
    let (running, error) = {
        let inst = node(&ctx, n_llm);
        let g = inst.runtime.lock().unwrap();
        match &*g {
            NodeRuntime::Llm { running, error, .. } => (*running, *error),
            _ => return Err("n_llm не Llm runtime".into()),
        }
    };

    let inst = node(&ctx, n_llm);
    llm::start(&inst, &ctx);

    let t0 = Instant::now();
    loop {
        drain();
        if let Some(e) = error.get_untracked() {
            return Err(format!("LLM: {e}"));
        }
        if !running.get_untracked() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    drain();
    if let Some(e) = error.get_untracked() {
        return Err(format!("LLM: {e}"));
    }

    // Прочитать ответ через граф (порт answer → TextView answer) и напрямую.
    refresh(&ctx);
    let answer = {
        let inst = node(&ctx, n_llm);
        let g = inst.runtime.lock().unwrap();
        match &*g {
            NodeRuntime::Llm { output_text, .. } => output_text.get_untracked(),
            _ => String::new(),
        }
    };
    eprintln!("llm_smoke: готово за {:.1}s", t0.elapsed().as_secs_f32());
    println!("─── PROMPT ───\n{prompt}\n─── ANSWER ───\n{answer}\n──────────────");
    if answer.trim().is_empty() {
        return Err("пустой ответ".into());
    }
    Ok(())
}

fn main() {
    syngui::signal::init_main_thread();
    if let Err(e) = run() {
        eprintln!("llm_smoke FAILED: {e}");
        std::process::exit(1);
    }
}
