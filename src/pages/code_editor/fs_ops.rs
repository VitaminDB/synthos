//! Файловые операции страницы редактора кода.
//!
//! Все функции синхронные через `std::fs` (по образцу `chat::storage`).
//! Любая ошибка ввода-вывода логируется через `eprintln!` и возвращает
//! пустой/None-результат — приложение никогда не паникует из-за проблем
//! с файловой системой (правило TASK.md).

use std::path::{Path, PathBuf};

use syngui::widgets::TreeNode;

use super::file_icons;

/// Префикс id у placeholder-leaf'ов под нераскрытыми папками.
/// Без хотя бы одного дочернего узла TreeView не рисует чеврон у branch'а
/// (`FlatNode.has_children = !children.is_empty()` в tree_view.rs:77),
/// поэтому каждой папке в момент создания подкладывается sentinel-leaf,
/// который заменяется реальным содержимым при первом раскрытии.
pub const PLACEHOLDER_PREFIX: &str = "__placeholder__:";

fn placeholder_for(parent_id: &str) -> TreeNode {
    TreeNode::leaf(format!("{PLACEHOLDER_PREFIX}{parent_id}"), "")
}

/// Стабильный id узла = абсолютный путь в виде строки. Используется и в
/// `tree_nodes`, и в callback'ах TreeView (`on_select`/`on_toggle`).
pub fn id_for(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// Прочитать ОДИН уровень содержимого папки в `Vec<TreeNode>` — единое
/// ядро для [`read_dir_to_nodes`] и [`read_dir_to_nodes_checked`].
/// Сорт: папки первыми в alpha-порядке, затем файлы в alpha-порядке.
/// Скрытые элементы (`.git`, dotfiles) не фильтруются — пользователь
/// видит всё содержимое каталога, как в Zed/VSCode по умолчанию.
///
/// **Fail-closed**: ЛЮБАЯ ошибка — `opendir` (EMFILE/ENFILE/ENOENT) ИЛИ
/// ошибка итерации / `file_type` для отдельной записи — возвращается как
/// `Err`. Частичный список НИКОГДА не отдаётся как `Ok`.
///
/// Зачем это критично: watcher-refresh использует свежий снимок, чтобы
/// решить, какие узлы дерева удалить (те, которых в снимке нет). Прежняя
/// версия делала `entries.flatten()` и `Err =>` skip у `file_type`, молча
/// отбрасывая записи с транзиентной ошибкой (частая ситуация во время
/// `git checkout`, массовых правок, исчерпания fd). Неполный снимок
/// принимался за истину → merge удалял из дерева ЖИВЫЕ файлы/папки.
/// Отсюда и баг «пропадает ветка / целое дерево». Теперь любой сбой —
/// `Err`, и вызывающий просто пропускает обновление (дерево не трогается).
fn read_level(path: &Path) -> std::io::Result<Vec<TreeNode>> {
    let mut dirs: Vec<(String, PathBuf)> = Vec::new();
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?; // ошибка итерации → Err, НЕ молчаливый skip
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type()?.is_dir() {
            dirs.push((name, p));
        } else {
            files.push((name, p));
        }
    }
    dirs.sort_by_key(|a| a.0.to_lowercase());
    files.sort_by_key(|a| a.0.to_lowercase());

    let mut nodes: Vec<TreeNode> = Vec::with_capacity(dirs.len() + files.len());
    for (name, p) in dirs {
        let id = id_for(&p);
        let placeholder = placeholder_for(&id);
        // Папки в свёрнутом виде. `MI_FOLDER` — закрытая иконка; пер-node
        // изменение на FolderOpen потребовало бы расширения TreeView API
        // (per-node класс / per-state icon override). Сейчас цвет общий
        // через `.code-editor-tree { icon-color: ... }` в MSS.
        let icon = file_icons::icon_for_path(&p, true, false);
        nodes.push(TreeNode::branch(id, name, vec![placeholder]).icon(icon));
    }
    for (name, p) in files {
        let icon = file_icons::icon_for_path(&p, false, false);
        nodes.push(TreeNode::leaf(id_for(&p), name).icon(icon));
    }
    Ok(nodes)
}

/// Прочитать один уровень, отдавая пустой `Vec` при любой ошибке.
/// Только для «наполняющих» путей (выбор корня, первое раскрытие папки),
/// где терять нечего: на этом уровне в дереве ещё нет данных. Для
/// watcher-refresh использовать НЕЛЬЗЯ — там нужен fail-closed
/// [`read_dir_to_nodes_checked`], иначе пустой результат сотрёт ветку.
pub fn read_dir_to_nodes(path: &Path) -> Vec<TreeNode> {
    read_level(path).unwrap_or_else(|e| {
        eprintln!("[code-editor] read_dir {:?}: {e}", path);
        Vec::new()
    })
}

/// Найти узел с указанным `id` в дереве (рекурсивный обход).
pub fn find_node<'a>(nodes: &'a [TreeNode], target_id: &str) -> Option<&'a TreeNode> {
    for node in nodes {
        if node.id == target_id {
            return Some(node);
        }
        if let Some(found) = find_node(&node.children, target_id) {
            return Some(found);
        }
    }
    None
}

/// Удалить узел с указанным `id` (вместе с его поддеревом) из tree.
/// Используется fs_watcher::reconcile_loaded_dirs для подчистки stale-узлов,
/// которые остались в дереве после пропущенных notify-событий.
/// Возвращает `true`, если узел найден и удалён.
pub fn remove_node(nodes: &mut Vec<TreeNode>, target_id: &str) -> bool {
    if let Some(pos) = nodes.iter().position(|n| n.id == target_id) {
        nodes.remove(pos);
        return true;
    }
    for node in nodes.iter_mut() {
        if remove_node(&mut node.children, target_id) {
            return true;
        }
    }
    false
}

/// Прочитать один уровень с явной ошибкой при ЛЮБОЙ проблеме. В отличие
/// от [`read_dir_to_nodes`], не глотает ошибки: ENOENT (папки больше нет),
/// EMFILE/ENFILE и ошибки итерации отдельных записей возвращаются как
/// `Err`. Нужно watcher'у, чтобы (а) отличить «папка пустая» от «папки
/// больше нет» и (б) НИКОГДА не принять неполное чтение за истину —
/// иначе merge удалит из дерева узлы, которые реально существуют
/// (см. [`read_level`]).
pub fn read_dir_to_nodes_checked(path: &Path) -> std::io::Result<Vec<TreeNode>> {
    // Метаданные сначала — read_dir по симлинку на удалённую папку может
    // вернуть Ok с пустым итератором, мы хотим ENOENT и в этом случае.
    let _ = std::fs::metadata(path)?;
    read_level(path)
}

/// Заменить детей узла `target_id` на `new_kids` и пометить его раскрытым.
/// `new_kids` — `Option`, чтобы рекурсия могла «забрать» Vec ровно один раз.
/// Возвращает `true`, если узел найден.
///
/// Свежие дети мержатся со старыми через [`merge_preserve_expanded`] — это
/// важно для fs_watcher-вызовов: иначе раскрытые подпапки целевого узла
/// потеряли бы свои children при каждом StructureChanged-событии. Для
/// lazy-load из `toggle_dir` поведение не меняется: там старые дети =
/// `[placeholder]`, у которого `expanded=false`, и merge сводится к
/// «вернуть свежие данные».
pub fn replace_children_and_expand(
    nodes: &mut [TreeNode],
    target_id: &str,
    new_kids: &mut Option<Vec<TreeNode>>,
) -> bool {
    for node in nodes.iter_mut() {
        if node.id == target_id {
            if let Some(kids) = new_kids.take() {
                node.children = merge_preserve_expanded(kids, &node.children);
                node.expanded = true;
                return true;
            }
        }
        if replace_children_and_expand(&mut node.children, target_id, new_kids) {
            return true;
        }
    }
    false
}

/// Объединить свежий one-level снимок (`new_kids`, как из
/// [`read_dir_to_nodes`]) со старым уровнем дерева (`old`), сохраняя
/// `expanded` и `children` для совпавших по `id` папок, которые в `old`
/// были раскрыты.
///
/// Зачем: fs_watcher / fs_actions перечитывают один уровень каталога с
/// диска и хотят отразить структурные изменения в `tree_nodes`. Если
/// тупо записать new_kids поверх старых children — все вложенные
/// раскрытые папки превратятся в placeholder-папки (их дети пропадут
/// из UI до следующего ручного раскрытия). Этот merge переносит
/// поддерево раскрытых ветвей из старого снимка.
///
/// Семантика:
/// - Узлы, которых нет в `new_kids` (удалены с диска), выпадают.
/// - Узлы, которых нет в `old` (новые на диске), попадают «как есть».
/// - Папки, совпавшие по `id`: если в `old` была `expanded=true` И
///   имела реальных детей (не один placeholder), переносим
///   `expanded=true` + `children` из `old` (имя/иконка/decoration —
///   свежие из `new_kids`).
/// - Все прочие случаи (свёрнутые в `old`, файлы) — берём свежие.
///
/// Порядок результата = порядок из `new_kids` (актуальная alpha-сорт
/// `dirs-then-files` из `read_dir_to_nodes`).
///
/// Merge НЕ рекурсивный: сохраняем старое поддерево «как есть». Если
/// внутри тоже что-то изменилось — fs_watcher пришлёт отдельный
/// StructureChanged для этого уровня, и он отработает свой merge.
pub fn merge_preserve_expanded(new_kids: Vec<TreeNode>, old: &[TreeNode]) -> Vec<TreeNode> {
    use std::collections::HashMap;
    let old_by_id: HashMap<&str, &TreeNode> =
        old.iter().map(|n| (n.id.as_str(), n)).collect();
    new_kids
        .into_iter()
        .map(|mut n| {
            if let Some(prev) = old_by_id.get(n.id.as_str()) {
                if prev.expanded && has_real_children(prev) {
                    n.expanded = true;
                    n.children = prev.children.clone();
                }
            }
            n
        })
        .collect()
}

fn has_real_children(node: &TreeNode) -> bool {
    node.children
        .iter()
        .any(|c| !c.id.starts_with(PLACEHOLDER_PREFIX))
}

/// Слить свежий one-level снимок `fresh` (как из [`read_dir_to_nodes`]) в
/// существующий уровень `existing` IN-PLACE, СОХРАНЯЯ пользовательское
/// состояние: раскрытость веток и их уже загруженные поддеревья.
///
/// Это refresh-функция для watcher'а. Её отличия от
/// [`merge_preserve_expanded`] / [`replace_children_and_expand`], которые
/// применялись на внешних изменениях раньше и давали два бага:
///  - **Не форсит `expanded`.** Раскрытость каждой совпавшей ветки
///    берётся из `existing` как есть. Свёрнутую пользователем папку
///    обновление ФС больше не раскрывает обратно.
///  - **Возвращает `true` только при реальном изменении** набора или
///    порядка узлов. Вызывающий на no-op НЕ трогает сигнал `tree_nodes`
///    → `TreeView` не пересоздаётся → скролл и выделение сохраняются.
///
/// Семантика набора:
///  - совпавшие по `id` узлы переносятся из `existing` ЦЕЛИКОМ (свой
///    `expanded`, `children`, иконка);
///  - узлы из `fresh`, которых не было в `existing` — добавляются как
///    есть (свёрнутыми, с placeholder для папок);
///  - узлы `existing`, которых нет в `fresh` (удалены с диска) — выпадают.
///
/// Порядок результата = порядок `fresh` (alpha dirs-then-files).
/// Вызывающий ОБЯЗАН передавать `fresh` только из доверенного чтения
/// (`Ok` из [`read_dir_to_nodes_checked`]); удаление узлов на основе
/// неполного снимка недопустимо.
pub fn merge_level_preserve_state(existing: &mut Vec<TreeNode>, fresh: Vec<TreeNode>) -> bool {
    use std::collections::HashMap;
    let old_ids: Vec<String> = existing.iter().map(|n| n.id.clone()).collect();
    let mut old_by_id: HashMap<String, TreeNode> =
        existing.drain(..).map(|n| (n.id.clone(), n)).collect();

    let mut out: Vec<TreeNode> = Vec::with_capacity(fresh.len());
    for fresh_node in fresh {
        match old_by_id.remove(&fresh_node.id) {
            Some(prev) => out.push(prev), // сохраняем expanded/children как есть
            None => out.push(fresh_node), // новый узел — свёрнут
        }
    }
    // changed, если набор или порядок отличаются от исходного: удаления →
    // разная длина; добавления/переименования → разный набор; смена
    // сортировки → разный порядок.
    let changed = out.len() != old_ids.len()
        || out.iter().zip(old_ids.iter()).any(|(n, old)| &n.id != old);
    *existing = out;
    changed
}

/// Найти узел `target_id` и слить свежий снимок его детей через
/// [`merge_level_preserve_state`], сохраняя раскрытость самого узла и его
/// поддеревьев. В отличие от [`replace_children_and_expand`] (который
/// форсит `expanded = true` и нужен только для явного toggle), эта
/// функция НЕ меняет раскрытость — для фонового watcher-refresh.
///
/// `fresh` — `Option`, чтобы рекурсия «забрала» Vec ровно один раз.
/// Возвращает `Some(changed)` если узел найден (`changed` — изменились ли
/// дети), `None` если узла в дереве нет.
pub fn refresh_children_preserve_state(
    nodes: &mut [TreeNode],
    target_id: &str,
    fresh: &mut Option<Vec<TreeNode>>,
) -> Option<bool> {
    for node in nodes.iter_mut() {
        if node.id == target_id {
            if let Some(kids) = fresh.take() {
                return Some(merge_level_preserve_state(&mut node.children, kids));
            }
        }
        if let Some(changed) =
            refresh_children_preserve_state(&mut node.children, target_id, fresh)
        {
            return Some(changed);
        }
    }
    None
}

/// Установить `expanded = value` на узле с указанным `id`. Возвращает
/// `true`, если узел найден.
pub fn set_node_expanded(nodes: &mut [TreeNode], target_id: &str, value: bool) -> bool {
    for node in nodes.iter_mut() {
        if node.id == target_id {
            node.expanded = value;
            return true;
        }
        if set_node_expanded(&mut node.children, target_id, value) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder_with_placeholder(id: &str) -> TreeNode {
        let ph = TreeNode::leaf(format!("{PLACEHOLDER_PREFIX}{id}"), "");
        TreeNode::branch(id, id, vec![ph])
    }

    fn expanded_folder(id: &str, children: Vec<TreeNode>) -> TreeNode {
        let mut n = TreeNode::branch(id, id, children);
        n.expanded = true;
        n
    }

    #[test]
    fn merge_preserves_expanded_folder_children() {
        let old = vec![expanded_folder(
            "a",
            vec![TreeNode::leaf("a/x", "x"), TreeNode::leaf("a/y", "y")],
        )];
        let new_kids = vec![folder_with_placeholder("a")];

        let out = merge_preserve_expanded(new_kids, &old);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "a");
        assert!(out[0].expanded, "expanded должен сохраниться");
        assert_eq!(out[0].children.len(), 2);
        assert_eq!(out[0].children[0].id, "a/x");
        assert_eq!(out[0].children[1].id, "a/y");
    }

    #[test]
    fn merge_drops_removed_nodes() {
        let old = vec![
            expanded_folder("a", vec![TreeNode::leaf("a/x", "x")]),
            TreeNode::leaf("b", "b"),
        ];
        let new_kids = vec![folder_with_placeholder("a")];

        let out = merge_preserve_expanded(new_kids, &old);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "a");
    }

    #[test]
    fn merge_adds_new_nodes_collapsed() {
        let old = vec![expanded_folder("a", vec![TreeNode::leaf("a/x", "x")])];
        let new_kids = vec![
            folder_with_placeholder("a"),
            TreeNode::leaf("new_file.rs", "new_file.rs"),
        ];

        let out = merge_preserve_expanded(new_kids, &old);
        assert_eq!(out.len(), 2);
        // a — сохранён с детьми
        assert!(out[0].expanded);
        assert_eq!(out[0].children[0].id, "a/x");
        // new_file — свежий leaf
        assert_eq!(out[1].id, "new_file.rs");
        assert!(!out[1].expanded);
    }

    #[test]
    fn merge_keeps_collapsed_folders_collapsed() {
        let old = vec![folder_with_placeholder("a")];
        let new_kids = vec![folder_with_placeholder("a")];

        let out = merge_preserve_expanded(new_kids, &old);
        assert_eq!(out.len(), 1);
        assert!(!out[0].expanded);
        assert_eq!(out[0].children.len(), 1);
        assert!(out[0].children[0].id.starts_with(PLACEHOLDER_PREFIX));
    }

    #[test]
    fn merge_does_not_preserve_when_old_has_only_placeholder() {
        // Edge: expanded=true, но children = [placeholder] (странный
        // переходный state). Берём свежие данные, иначе UI застынет с
        // placeholder вместо настоящего содержимого.
        let mut weird = folder_with_placeholder("a");
        weird.expanded = true;
        let old = vec![weird];
        let new_kids = vec![folder_with_placeholder("a")];

        let out = merge_preserve_expanded(new_kids, &old);
        assert_eq!(out.len(), 1);
        // expanded=false: свежие данные, ничего не переносим
        assert!(!out[0].expanded);
    }

    #[test]
    fn remove_node_drops_subtree_at_any_depth() {
        let mut tree = vec![expanded_folder(
            "a",
            vec![
                expanded_folder("a/b", vec![TreeNode::leaf("a/b/x", "x")]),
                TreeNode::leaf("a/y", "y"),
            ],
        )];

        // top-level
        assert!(remove_node(&mut tree, "a/b"));
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].id, "a/y");

        // не существует — no-op, не паникует
        assert!(!remove_node(&mut tree, "nope"));

        // удалить корневой
        assert!(remove_node(&mut tree, "a"));
        assert!(tree.is_empty());
    }

    #[test]
    fn replace_children_and_expand_merges_subtree() {
        // a/ expanded, внутри b/ expanded со своими детьми. fs_watcher
        // прислал StructureChanged(a) → перечитал один уровень a/ →
        // получил [b (placeholder)]. После merge b должен остаться
        // expanded со старыми детьми.
        let mut tree = vec![expanded_folder(
            "a",
            vec![expanded_folder(
                "a/b",
                vec![TreeNode::leaf("a/b/x", "x")],
            )],
        )];
        let new_kids = vec![folder_with_placeholder("a/b")];
        let mut taken = Some(new_kids);

        let replaced = replace_children_and_expand(&mut tree, "a", &mut taken);
        assert!(replaced);
        assert!(tree[0].expanded);
        let b = &tree[0].children[0];
        assert_eq!(b.id, "a/b");
        assert!(b.expanded, "вложенная раскрытая папка не должна терять expanded");
        assert_eq!(b.children.len(), 1);
        assert_eq!(b.children[0].id, "a/b/x");
    }

    // ── merge_level_preserve_state (watcher-refresh) ──────────────────────

    fn collapsed_loaded_folder(id: &str, children: Vec<TreeNode>) -> TreeNode {
        // expanded=false, но дети РЕАЛЬНЫЕ (папка была раскрыта, потом
        // свёрнута пользователем — содержимое осталось в памяти).
        let mut n = TreeNode::branch(id, id, children);
        n.expanded = false;
        n
    }

    #[test]
    fn merge_level_keeps_user_collapsed_folder_collapsed() {
        // Регрессия: «свернул ветку → изменение ФС → опять раскрылась».
        // Refresh НЕ должен форсить expanded.
        let mut existing =
            vec![collapsed_loaded_folder("a", vec![TreeNode::leaf("a/x", "x")])];
        let fresh = vec![folder_with_placeholder("a")];

        let changed = merge_level_preserve_state(&mut existing, fresh);
        assert!(!changed, "набор тот же — должно быть no-op");
        assert_eq!(existing.len(), 1);
        assert!(!existing[0].expanded, "свёрнутая ветка осталась свёрнутой");
        assert_eq!(existing[0].children.len(), 1, "дети сохранены");
        assert_eq!(existing[0].children[0].id, "a/x");
    }

    #[test]
    fn merge_level_keeps_expanded_folder_expanded() {
        let mut existing = vec![expanded_folder("a", vec![TreeNode::leaf("a/x", "x")])];
        let fresh = vec![folder_with_placeholder("a")];

        let changed = merge_level_preserve_state(&mut existing, fresh);
        assert!(!changed);
        assert!(existing[0].expanded);
        assert_eq!(existing[0].children[0].id, "a/x");
    }

    #[test]
    fn merge_level_noop_when_unchanged_returns_false() {
        // Ключ к фиксу скролла: одинаковый набор → false → вызывающий не
        // трогает сигнал → TreeView не пересоздаётся → скролл сохраняется.
        let mut existing = vec![TreeNode::leaf("a", "a"), TreeNode::leaf("b", "b")];
        let fresh = vec![TreeNode::leaf("a", "a"), TreeNode::leaf("b", "b")];

        assert!(!merge_level_preserve_state(&mut existing, fresh));
        assert_eq!(existing.len(), 2);
    }

    #[test]
    fn merge_level_detects_add_and_remove() {
        let mut existing = vec![TreeNode::leaf("a", "a"), TreeNode::leaf("b", "b")];
        // b удалён, c добавлен.
        let fresh = vec![TreeNode::leaf("a", "a"), TreeNode::leaf("c", "c")];

        assert!(merge_level_preserve_state(&mut existing, fresh));
        let ids: Vec<&str> = existing.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c"]);
    }

    #[test]
    fn refresh_children_preserve_state_does_not_force_expand() {
        // a раскрыта, внутри b СВЁРНУТА (с загруженными детьми). Refresh
        // детей a по событию ФС не должен раскрыть b обратно.
        let b = collapsed_loaded_folder("a/b", vec![TreeNode::leaf("a/b/x", "x")]);
        let mut tree = vec![expanded_folder("a", vec![b])];
        let fresh = vec![folder_with_placeholder("a/b")];

        let changed = refresh_children_preserve_state(&mut tree, "a", &mut Some(fresh));
        assert_eq!(changed, Some(false));
        let b_after = &tree[0].children[0];
        assert_eq!(b_after.id, "a/b");
        assert!(!b_after.expanded, "свёрнутая вложенная ветка осталась свёрнутой");
        assert_eq!(b_after.children[0].id, "a/b/x");
    }

    #[test]
    fn refresh_children_returns_none_for_missing_node() {
        let mut tree = vec![TreeNode::leaf("a", "a")];
        let mut fresh = Some(vec![TreeNode::leaf("x", "x")]);
        assert_eq!(
            refresh_children_preserve_state(&mut tree, "nope", &mut fresh),
            None
        );
    }

    #[test]
    fn read_dir_checked_missing_path_is_not_found() {
        // fail-closed: несуществующий путь → Err(NotFound), а не Ok([]).
        // reconcile полагается на это, чтобы удалять только реально
        // удалённые каталоги (а не на любой ошибке чтения).
        let err =
            read_dir_to_nodes_checked(Path::new("/nonexistent/synthos/code-editor/xyz"))
                .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
