use std::path::PathBuf;

use syngui::prelude::*;
use syngui::widgets::{Reactive, Row, ToolButton};

use crate::icons::MI_FOLDER_OPEN;

pub fn node_file_picker(
    tooltip: impl Into<String>,
    sig: RwSignal<Option<PathBuf>>,
    filters: &'static [(&'static str, &'static [&'static str])],
    on_pick: impl Fn(&PathBuf) + Send + Sync + 'static,
) -> Box<dyn Widget> {
    node_file_picker_placeholder(tooltip, sig, filters, || tr!("nodes.file_picker.empty"), on_pick)
}

/// То же, но с текстом-плейсхолдером для пустого сигнала — когда «файл не
/// выбран» не значит «ничего не будет»: например, override-поля ACE-Step
/// Checkpoint показывают дефолтное имя бандла, которое возьмётся из каталога.
///
/// Плейсхолдер — замыкание, потому что считается ВНУТРИ `Reactive`: если оно
/// читает сигналы (каталог моделей), подпись сама обновится при их смене.
pub fn node_file_picker_placeholder(
    tooltip: impl Into<String>,
    sig: RwSignal<Option<PathBuf>>,
    filters: &'static [(&'static str, &'static [&'static str])],
    placeholder: impl Fn() -> String + Send + Sync + 'static,
    on_pick: impl Fn(&PathBuf) + Send + Sync + 'static,
) -> Box<dyn Widget> {
    let title = tooltip.into();
    let pick_btn = ToolButton::new(MI_FOLDER_OPEN)
        .tooltip(title.clone())
        .on_click(move || {
            let mut dlg = rfd::FileDialog::new().set_title(&title);
            for (label, exts) in filters {
                let name = syngui::i18n::try_tr(label).unwrap_or_else(|| label.to_string());
                dlg = dlg.add_filter(&name, exts);
            }
            if let Some(p) = dlg.pick_file() {
                on_pick(&p);
                sig.set(Some(p));
            }
        })
        .class("node-file-picker-btn");

    let filename_text = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let widget: Box<dyn Widget> = match sig.get() {
            Some(p) => {
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string_lossy().to_string());
                Box::new(Text::new(name).class("node-file-picker-name"))
            }
            None => Box::new(Text::new(placeholder()).class("node-file-picker-empty")),
        };
        vec![widget]
    });

    Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("node-file-picker-row")
            .children(vec![Box::new(pick_btn) as Box<dyn Widget>, Box::new(filename_text)]),
    )
}
