use syngui::prelude::*;
use syngui::widgets::{Dropdown, DropdownItem};

use super::utils::idx_in;

pub fn node_dropdown_field(
    options: &'static [&'static str],
    idx: RwSignal<usize>,
) -> Box<dyn Widget> {
    let items: Vec<DropdownItem> = options.iter().map(|s| DropdownItem::simple(*s)).collect();
    let current = options
        .get(idx.get_untracked())
        .copied()
        .unwrap_or_else(|| options.first().copied().unwrap_or(""))
        .to_string();
    Box::new(
        Dropdown::with_items(items)
            .selected(current)
            .on_change(move |s| {
                if let Some(i) = idx_in(options, &s) {
                    idx.set(i);
                }
            })
            .class("node-input-dropdown"),
    )
}
