//! Построение `Vec<TreeNode>` из содержимого `Bundle` через
//! `list_dir_shallow` рекурсивно. Иконки — material через
//! `pages::code_editor::file_icons` (одинаковый визуальный язык по всему
//! приложению).

use std::path::Path;

use syngui::widgets::TreeNode;
use synaptix_bundle::{Bundle, DirEntry};

use crate::pages::code_editor::file_icons;

/// Построить дерево содержимого пакета.
///
/// Имена файлов внутри `.syn` — POSIX-style относительные пути (см.
/// `synaptix_bundle::path::normalize`). Группируем их в TreeNode'ы по сегментам
/// через `Bundle::list_dir_shallow(prefix)`. Папки изначально свёрнуты
/// (`expanded=false`); на верхнем уровне раскрыты — UX-комфорт.
pub fn build_dir_tree(bundle: &Bundle) -> Vec<TreeNode> {
    build_dir_recursive(bundle, "", 0)
}

fn build_dir_recursive(bundle: &Bundle, prefix: &str, depth: usize) -> Vec<TreeNode> {
    let mut nodes: Vec<TreeNode> = Vec::new();
    for entry in bundle.list_dir_shallow(prefix) {
        match entry {
            DirEntry::Subdir(name) => {
                let full_prefix = if prefix.is_empty() {
                    name.to_string()
                } else {
                    format!("{prefix}/{name}")
                };
                let children = build_dir_recursive(bundle, &full_prefix, depth + 1);
                // На верхнем уровне дерева раскрываем первый уровень папок —
                // пользователь сразу видит контекст.
                let expanded = depth == 0;
                let path_for_icon = Path::new(name);
                let icon = file_icons::icon_for_path(path_for_icon, true, expanded);
                let node = TreeNode::branch(full_prefix.clone(), name.to_string(), children)
                    .icon(icon)
                    .expanded(expanded);
                nodes.push(node);
            }
            DirEntry::File(file) => {
                let display = leaf_name(&file.name);
                let path_for_icon = Path::new(&display);
                let icon = file_icons::icon_for_path(path_for_icon, false, false);
                nodes.push(TreeNode::leaf(file.name.clone(), display).icon(icon));
            }
        }
    }
    nodes
}

fn leaf_name(full: &str) -> String {
    full.rsplit('/').next().unwrap_or(full).to_string()
}
