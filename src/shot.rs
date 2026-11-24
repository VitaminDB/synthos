//! Снимок страницы редактора нод без окна (`synthos --screenshot out.png`):
//! проверка вёрстки нод, портов и проводов до и после правок без живого
//! сеанса. Граф — встроенный шаблон (`--template <id>`) или демонстрационный
//! набор встроенных нод (`--template demo`, по умолчанию) или открытые графы
//! пользователя (`--template workspace[:N]`, N-й граф).

use syngui::core::Point;
use syngui::prelude::*;

use crate::pages::node_editor::tabs::EditorWorkspace;
use crate::pages::node_editor::types::{Connection, NodeKind};

/// Параметры снимка.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub out: String,
    /// Id встроенного шаблона, `demo` или `workspace`.
    pub template: String,
    pub size: (u32, u32),
    pub scale: f64,
}

/// Демонстрационный граф: ноды разных видов (звук, текст, картинки, ядро
/// synaptix-flow), связанные проводами.
fn demo(ws: &EditorWorkspace) {
    let id = ws.new_untitled();
    let Some(tab) = ws.tab(id) else { return };
    let ctx = tab.ctx;
    let at = |x: f32, y: f32| Point::new(x, y);
    let file = ctx.add_node(NodeKind::AudioFile, at(40.0, 40.0));
    let gain = ctx.add_node(NodeKind::Gain, at(640.0, 40.0));
    let player = ctx.add_node(NodeKind::AudioPlayer, at(1240.0, 40.0));
    ctx.add_node(NodeKind::Equalizer6, at(40.0, 420.0));
    let mixer = ctx.add_node(NodeKind::Mixer, at(640.0, 420.0));
    ctx.set_mixer_n_inputs(mixer, 3);
    ctx.add_node(NodeKind::MarkdownView, at(1900.0, 40.0));
    let text = ctx.add_node(NodeKind::FlowText, at(1240.0, 420.0));
    let llm = ctx.add_node(NodeKind::FlowLlm, at(1900.0, 420.0));
    ctx.add_node(NodeKind::ImageLoad, at(40.0, 900.0));
    ctx.add_node(NodeKind::Number, at(640.0, 900.0));
    let mut conns = ctx.connections.get_untracked();
    for (from_node, from_port, to_node, to_port) in
        [(file, "out", gain, "in"), (gain, "out", player, "in"), (text, "text", llm, "prompt")]
    {
        conns.push(Connection { from_node, from_port, to_node, to_port });
    }
    ctx.connections.set(conns);
    ctx.pan.set(Point::new(0.0, 0.0));
    ctx.zoom.set(0.6);
}

/// Нарисовать страницу нод в PNG.
pub fn run(shot: &Shot) -> std::result::Result<(u32, u32), String> {
    let (theme_mss, _) = crate::build_context();
    let mss = format!("{}\n{}", crate::styles::styles(), theme_mss.get_untracked());
    let template = shot.template.clone();
    syngui::shot::render_png(
        move || {
            let (_, ctx) = crate::build_context();
            crate::i18n::install(ctx.general);
            provide_context(ctx);
            // `workspace` — открытые графы пользователя (только чтение: автосохранение
            // в снимке не ставится).
            let ws = if template.starts_with("workspace") { EditorWorkspace::new_or_restore() } else { EditorWorkspace::new() };
            provide_context(ws);
            provide_context(crate::syn_chat::SynChatCtx::new());
            provide_context(crate::syn_chat::SynModelRegistry::new());
            crate::models::install();
            // Шапка страницы (panel_header) ищет контекст глобального поиска.
            crate::search::provide();
            if template == "demo" {
                demo(&ws);
            } else if let Some(rest) = template.strip_prefix("workspace") {
                // `workspace:N` — N-й граф (с нуля), в том числе скрытый агентский.
                let n: usize = rest.trim_start_matches(':').parse().unwrap_or(0);
                if let Some(tab) = ws.tabs.get_untracked().get(n) {
                    ws.active.set(Some(tab.id));
                }
            } else {
                match crate::templates::list_all().into_iter().find(|t| t.id == template) {
                    Some(t) => {
                        ws.open_template(&t);
                    }
                    None => eprintln!("synthos --screenshot: нет шаблона {template}"),
                }
            }
            Box::new(DecoratedBox::new().class("grow").child(crate::pages::node_editor::view()))
        },
        &mss,
        shot.size,
        shot.scale,
        &shot.out,
    )
}
