# Code editor: git-status декорации (Zed-style)

В дереве файлов редактора кода synthos (`pages/code_editor`) каждый узел подсвечивается в соответствии с git-статусом:

- **Modified** — `#FFB454` (yellow). Файл: жёлтые иконка и текст. Папка-предок: жёлтая точка в правом нижнем углу иконки.
- **New** (untracked) — `#6BD36B` (green). Аналогично.
- **Deleted** — `#EE5E48` (red). Удалённый файл показывается как **ghost-нода** (зачёркнутый красный текст). Папки получают красную точку.
- **Conflict** — `#B07FFF` (purple).

Цвета зеркалятся в двух местах одновременно: `git_status::GitPalette::default()` (Rust) и `--git-color-*` (`styles/components/code_editor.mss`). Менять — обе стороны.

Приоритет для rollup на папки: `Conflict > Modified > New > Deleted`.

## Архитектура

```
┌──────────────────────────────────────────────────────────────────┐
│  fs_watcher (notify, std::thread)                                │
│  • workdir файлы → request_git_refresh                           │
│  • .git/HEAD, .git/index, .git/refs/* → request_git_refresh     │
└──────────────┬───────────────────────────────────────────────────┘
               │ request_refresh()
               ▼
┌──────────────────────────────────────────────────────────────────┐
│  GitStatusWorker (std::thread + mpsc + 500мс debounce)           │
│  • git_status::compute(root) через gix::Repository::status       │
│  • run_on_main_thread → session.git_status.set(Arc<Map>)         │
└──────────────┬───────────────────────────────────────────────────┘
               │ signal change
               ▼
┌──────────────────────────────────────────────────────────────────┐
│  file_tree::body() Reactive                                      │
│  • git_status::apply_to_nodes(tree, &git, &palette)              │
│    – навешивает TreeNodeDecoration на узлы                       │
│    – injects ghost-ноды для Deleted-файлов в развёрнутые папки  │
│  • TreeView рендерит decoration:                                 │
│    – label_color/icon_color → жёлтый/зелёный/фиолетовый/красный │
│    – badge_color → круг 7px в правом нижнем углу иконки          │
│    – strikethrough → line-through на лейбле                      │
└──────────────────────────────────────────────────────────────────┘
```

## Ключевые файлы

- `syngui/src/widgets/data/tree_view.rs` — `TreeNodeDecoration` (label/icon color, badge, strikethrough). Badge рисуется как Material Icons `\u{EF4A}` (circle) через `push_text` — НЕ через `push_rect`, потому что batcher всегда отрисовывает Rect (shader_order=2) до Text (=5), и rect-badge оказывался под иконкой. Глиф vs глиф в одном Text-батче рисуется в insertion-order, поэтому badge-after-icon гарантированно сверху. Halo (тонкая обводка цветом фона) — такой же circle-глиф чуть большего размера. Геометрия: `BADGE_DIAMETER_RATIO=0.55`, `BADGE_HALO_RATIO=0.08` (всё выводится из `ICON_GLYPH_SIZE=16`, без хардкодов).
- `app/synthos/src/pages/code_editor/git_status.rs` — `GitStatus`, `GitStatusMap`, `compute`, `compute_folder_rollup`, `apply_to_nodes`, `inject_ghost_deleted`, `GitPalette`, `GitStatusWorker`. 11 unit-тестов + 1 integration-smoke на реальном workspace.
- `app/synthos/src/pages/code_editor/state.rs` — `CodeSession::git_status: RwSignal<Arc<GitStatusMap>>`, `git_worker_registry`, `install_git_worker`/`drop_git_worker`/`request_git_refresh`. Воркер стартует параллельно с fs-watcher в `spawn_git_worker`.
- `app/synthos/src/pages/code_editor/fs_watcher.rs` — `is_git_meta_event` (whitelist в `.git/`), `FsEvent::GitMetaChanged`, `request_git_refresh` в `apply_batch`.
- `app/synthos/src/pages/code_editor/file_tree.rs` — подписка на `git_status` в `body()`, ghost-handling в `handle_select`/`handle_toggle` (показывает notice «Файл удалён»).
- `app/synthos/styles/components/code_editor.mss` — `--git-color-*` токены.

## Изменения во фреймворке

`syngui::widgets::TreeNode` получил поле:

```rust
pub decoration: Option<TreeNodeDecoration>,
```

С builder-методами `.label_color(c)`, `.badge(c)`, `.strikethrough(on)`, `.decoration(deco)`. Это снимает known-limitation, отмеченное в коммите от 2026-04-28: TreeView теперь поддерживает per-node визуальные override-ы. Старый код продолжает работать без изменений (поле опционально, default = None).

## Будущие улучшения

- **Rename detection**: сейчас выключен (gix без `rewrites` опции). Включить = детектить move'ы, отображать одним «изменённым» цветом.
- **Source Control панель**: целое отдельное представление с полным `git status --porcelain=v2`-style списком + diff preview. `GitStatusMap` уже содержит данные для этого.
- **Per-file-type декорации в TreeView**: сейчас `.file-icon-*` классы применяются только в open_files и editor header. Чтобы дерево тоже было цветным по типу — потребуется либо `per-node class` в TreeNode (более глубокая правка фреймворка), либо отдельный pass `apply_file_kind_colors` в `file_tree::body`, который ставит `label_color` через decoration по типу файла. Конфликт с git: при наличии git-статуса тот побеждает.
- **Stage/unstage actions**: правый клик на ghost-ноде → restore. Правый клик на modified → stage/discard. Требует API gix для index-mutation.
- **Remote tracking**: ahead/behind отображение на корне сессии (через `repo.head().peel_to_commit()` и сравнение с upstream).

## Тесты

```bash
cargo test -p syngui --lib widgets::data::tree_view
cargo test -p synthos --lib pages::code_editor::git_status
cargo test -p synthos --test git_status_smoke
```
