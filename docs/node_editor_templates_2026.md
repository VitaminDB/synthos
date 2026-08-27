# Node Editor: Templates / Multi-tab / Run-controls (2026-05-10, synthos 0.1.77)

> **2026-08-27.** Полосы вкладок на странице больше нет: открытые графы —
> плитки нав-рейла, Run-пилюля и кнопки шаблонов — в общей шапке, выбор
> шаблона всегда создаёт новую копию. См. `docs/workspace_shell_2026.md`;
> ниже — исходное описание, модель `EditorWorkspace`/`Template`/CRUD актуальна.

Расширение `pages/node_editor` до ComfyUI-подобного workflow:
- **Левая Templates-панель** (collapsible) с builtin + custom шаблонами.
- **Multi-tab editor** в стилистике терминальных вкладок (Zed/VSCode-like).
- **Floating Run/Pause/Stop pill** в правом-верхнем углу canvas.
- **Padding-frame** вокруг dot-grid (точки больше не цепляются к границам).

## Архитектура

```
shell.Row [
  nav_rail,
  templates_panel (collapsible, AnimatedSize/Width),  ← новое
  RouterView (route="nodes" → node_editor::view)
]

node_editor::view = Column [
  TabsBar (open tabs, стиль .ne-tab-*)
  canvas-area (Reactive по active_ctx) {
    Stack [
      .ne-canvas-frame (padding 16, rounded 12)
        └ PanZoomViewport (dot-grid)
            └ world_layer (Reactive: wires + nodes)
      overlay_row [Toolbar | Spacer | RunControls]
      PopupMenu (Add Node)
    ]
  }
]
```

## Ключевые типы

### `EditorWorkspace` (`pages/node_editor/tabs.rs`)
```rust
pub struct EditorWorkspace {
    pub tabs: RwSignal<Vec<OpenTab>>,
    pub active: RwSignal<Option<TabId>>,
    pub run_state: RwSignal<RunState>,        // Stopped | Running | Paused
    pub templates_open: RwSignal<bool>,
    pub next_tab_id: RwSignal<u64>,
}
```
- `provide_context(EditorWorkspace::new())` в `lib.rs::run_desktop`.
- `OpenTab { id, title, source: Option<TemplateId>, dirty, ctx: NodeEditorCtx }`.
- `find_tab_by_template(tabs, id)` / `next_untitled_name(tabs)` — pure helpers.

### `Template` (`templates/model.rs`)
```rust
pub struct Template {
    #[serde(skip)] pub id: String,         // = filename без .json
    #[serde(skip)] pub builtin: bool,      // true для in-code шаблонов
    pub name: String,
    pub description: String,
    pub kind: TemplateKind,                // Full | Subgraph
    pub nodes: Vec<NodeData>,
    pub connections: Vec<ConnData>,
    pub viewport: Option<ViewportData>,
}
```

### Хранение
- **Builtin**: `templates/builtin.rs::all()` — генерируется в коде, read-only.
  Текущий набор: `Empty`, `Simple Add`, `Audio Chain (stub)`.
- **Custom**: `~/.config/synthos/templates/<slug>.json` — JSON pretty-printed,
  slug-based filenames, коллизии резолвятся `make_slug` (общий с skills).

CRUD: `templates::create / save / rename / duplicate_to_custom / delete`.
`bump_revision()` после CRUD триггерит реактивный re-render списка в панели.

## UI-классы (MSS)

| Файл | Классы |
|------|--------|
| `node_editor.mss` | `.node-editor-root`, `.ne-canvas-area`, `.ne-canvas-frame`, `.node-editor-viewport` (rounded + inset shadow), `.ne-empty*` |
| `node_editor_tabs.mss` | `.ne-tabs-bar`, `.ne-tab[--active]`, `.ne-tab-icon/title/close/add` |
| `node_editor_run_controls.mss` | `.ne-run-controls[--running/paused/stopped]`, `.ne-run-btn[--play/pause/stop][--active]`, `@keyframes ne-run-pulse-green` |
| `templates_panel.mss` | `.ne-templates-host[--open/closed]`, `.ne-templates-handle*`, `.ne-templates-panel`, `.ne-template-card*`, `.ne-template-chip[--full/subgraph]`, `.ne-template-badge` |

## Поведение

### Открытие шаблона
- Двойной клик по карточке (или `ContextMenu → "Открыть"`):
  `workspace.open_template(&t)`.
- Если шаблон уже открыт — переключается на существующую вкладку
  (через `find_tab_by_template`).
- Иначе — создаётся новый `OpenTab`, в его `NodeEditorCtx` загружается
  содержимое через `templates::convert::load_into_ctx`.

### Сохранение текущего графа
`+` в header панели → `save_current_as_template()`:
- Снимок активного `NodeEditorCtx` через `templates::convert::snapshot()`.
- `Template::empty("My template", Full)` + `nodes/connections/viewport`.
- `templates::create(t)` — slug-based id с защитой от коллизий.
- Notification «Сохранено: ...» через `app.notifications.success`.
- `bump_revision()` → панель перерисовывается с новым шаблоном.

### Inline-rename
Двойной клик по имени карточки (custom-only) → `editing.set(true)`. TextField
с initial-value, on_submit вызывает `templates::rename(old_id, new_name)`,
поле возвращается обратно в Text. Builtin-шаблоны игнорируют click.

### Удаление
`ContextMenu → "Удалить"` → `delete_open.set(true)` → `Dialog` с двумя
кнопками. На Confirm: `templates::delete(id, builtin=false)` +
`bump_revision()`.

### Run-controls
RwSignal `run_state` — пока декоративный (UI + Snackbar). Реальная
интеграция с `eval.rs` (gate auto-eval по `Stopped`/`Paused`, time-based
ticks для `Running`) — TODO когда появятся audio/animation-ноды.

Pulse-keyframe `ne-run-pulse-green` запускается на `.ne-run-btn--play.ne-run-btn--active`.

## Файлы

### Новые
- `src/templates/{mod,model,storage,convert,builtin}.rs` — JSON-CRUD + serde.
- `src/pages/node_editor/{tabs,tabs_bar,run_controls,template_preview}.rs`.
- `src/components/templates_panel/{mod,template_card,editable_label}.rs`.
- `styles/components/{node_editor_tabs,node_editor_run_controls,templates_panel}.mss`.

### Изменены
- `src/pages/node_editor/mod.rs` — мульти-таб view, overlay-row, frame.
- `src/pages/node_editor/types.rs` — `Serialize/Deserialize` на `NodeKind`.
- `src/lib.rs` — `provide_context(EditorWorkspace::new())`, panel в shell-Row.
- `src/components/mod.rs` — `pub mod templates_panel`.
- `src/styles.rs` — подключение трёх новых MSS-файлов.
- `Cargo.toml` — bump 0.1.76 → 0.1.77.

## Что вне scope этой итерации (TODO)

- **Subgraph templates**: select-rectangle на canvas → group-as-template.
- **Drag-and-drop** шаблона из панели на canvas (помимо двойного клика).
- **Drag-reorder вкладок** в TabsBar.
- **Time-based runtime** (animation tick) для realtime-нод; Run/Pause/Stop
  тогда станут не декоративными.
- **PNG-снимок canvas** как override для preview (сейчас preview всегда
  авто-генерируется из nodes/connections).
