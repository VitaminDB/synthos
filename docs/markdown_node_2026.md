# MarkdownView-нода в node-editor (2026-05-12)

Декоративная нода-аннотация. Без портов и executor-логики; визуально вписана в общий стиль карточек редактора нод.

## Ключевые свойства
- **Прозрачность** через MSS `opacity: 0.8` (при hover поднимается до `0.95`).
- **Resizable**: 8 handle'ов на углах/гранях, делегировано готовому `syngui::widgets::TransformBox`.
- **Два независимых режима** переключаются из ContextMenu по правому клику:
  - «Редактировать» → `MarkdownEditor` (text editing) ↔ «Просмотр» (по умолчанию) → `MarkdownView`.
  - «Изменить размер» → `TransformBox::active = true`, рисуются handle'ы и принимается drag.
- Drag всей ноды по header'у — стандартный `DragHandle`, как у всех нод; resize-режим не мешает.

## Файлы
- `app/synthos/src/pages/node_editor/types.rs` — `NodeKind::MarkdownView` + `NodeRuntime::MarkdownView { content, edit_mode, resize_mode, size }`.
- `app/synthos/src/pages/node_editor/registry.rs` — `MARKDOWN_VIEW: NodeKindMeta` (категория `Misc`, подкатегория `"Аннотации"`, без портов, `inline_ports=true`), `default_runtime` ветка с `default_markdown_content()`.
- `app/synthos/src/pages/node_editor/nodes/markdown_view.rs` — `MarkdownViewExec` (no-op), `toggle_edit_mode`/`toggle_resize_mode`, `body(node)`: `TransformBox(active=resize_mode, size_signal=size, resizable=true, moveable=false, rotatable=false)` оборачивает `Reactive`, переключающий `MarkdownView`↔`MarkdownEditor`.
- `app/synthos/src/pages/node_editor/node_view.rs` — доп. класс `.markdown-node` к `.node-card` + 2 пункта ContextMenu (`MI_EDIT_NOTE`/`MI_ASPECT_RATIO`).
- `app/synthos/src/pages/node_editor/template_preview.rs` — slate-цвет в `node_color()` для preview шаблона.
- `app/synthos/styles/components/markdown_node.mss` — `opacity` + `--tb-*` переменные под палитру node-editor.
- `app/synthos/src/icons.rs` — добавлен `MI_ASPECT_RATIO`.

## Архитектурные заметки
- `TransformBox::active=false` → `intercepts_child_events()=false`, child получает все mouse-события (links/copy-code/text editing работают). `active=true` → handles ловят drag, child нечувствителен — это и есть «режим resize».
- Размер ноды диктуется `TransformBox::explicit_dimensions` (явные `(w, h)`), `.node-card { width: fit-content; height: fit-content; }` корректно учитывает header + body.
- Содержимое (`content`) и размеры (`size`) живут в `NodeRuntime`, поэтому не сериализуются в шаблоны нод (как и у Mixer/Equalizer). Сериализация content между сессиями шаблонов — задел для будущего.
- `default_markdown_content()` в `registry.rs` — приветственный текст с подсказкой по управлению режимами.
