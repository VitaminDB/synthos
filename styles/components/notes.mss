/* Режим «Заметки»: дерево vault, редактор, правая панель, плитка рейла. */

/* Разделители трёхпанельного каркаса. */
/* Единый стиль с `.syn-chat-h-split` / `.code-editor-h-split`:
 * 1px-линия в цвет границ, accent при drag'е. */
.notes-h-split {
    border-color: var(--border-soft);
    accent-color: var(--primary);
    divider-thickness: 1px;
}

/* ── Дерево «Содержимое» ──────────────────────────────────────── */
.notes-tree {
    padding: 6px;
}
/* Высота строки фиксирована: от неё считаются зоны дропа
 * «перед / внутрь / после» (contents.rs::ROW_H). */
.notes-tree-row {
    height: 28px;
    border-radius: 6px;
    padding: 0px 6px 0px 2px;
    transition: background-color 120ms ease-out;
}
.notes-tree-row:hover {
    background-color: var(--surface-hover);
}
.notes-tree-row.selected {
    background-color: var(--surface-selected);
}
.notes-tree-chevron-box {
    width: 16px;
    height: 20px;
}
.notes-tree-chevron {
    icon-size: 16px;
    color: var(--text-subtle);
}
.notes-tree-icon-box {
    width: 22px;
    height: 22px;
    border-radius: 5px;
}
.notes-tree-icon-box:hover {
    background-color: var(--surface-selected);
}
.notes-tree-icon {
    icon-size: 16px;
    font-size: 14px;
    color: var(--text-muted);
}
.notes-tree-icon.emoji {
    font-size: 14px;
    color: var(--text);
}
.notes-tree-name {
    font-size: 13px;
    color: var(--text);
}
.notes-tree-rename {
    height: 24px;
    font-size: 13px;
    padding: 0 6px;
    border-radius: 6px;
    border-width: 1.5px;
    border-color: var(--primary);
    background-color: var(--bg-search);
}
.notes-tree-tail {
    height: 40px;
}
.notes-tree-empty {
    padding: 16px 10px;
}
.notes-tree-empty-text {
    font-size: 12px;
    color: var(--text-muted);
}

/* Иконка страницы в шапке центра — кликабельна. */
.notes-header-icon-bubble:hover {
    background-color: var(--surface-selected);
}
.notes-header-icon.emoji {
    font-size: 16px;
    color: var(--text);
}

/* ── Центр: редактор ──────────────────────────────────────────── */
.notes-editor-page {
    background-color: transparent;
}
.notes-empty-icon {
    font-size: 44px;
    color: var(--text-subtle);
}
.notes-empty-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
}
.notes-empty-hint {
    font-size: 13px;
    color: var(--text-muted);
}

/* Палитра DocumentEditor из токенов темы (дефолты виджета — тёмные). */
document-editor {
    --doc-text-color: var(--text);
    --doc-muted-color: var(--text-muted);
    --doc-heading-color: var(--text);
    --doc-link-color: var(--primary);
    --doc-link-missing-color: var(--error);
    --doc-caret-color: var(--primary);
    --doc-selection-color: var(--surface-selected);
    --doc-code-bg: var(--surface-hover);
    --doc-code-color: var(--primary);
    --doc-code-block-bg: var(--surface-hover);
    --doc-code-block-color: var(--text);
    --doc-quote-border-color: var(--border-strong);
    --doc-bullet-color: var(--text-muted);
    --doc-number-color: var(--text-muted);
    --doc-checkbox-color: var(--text-subtle);
    --doc-checkbox-check-color: var(--primary);
    --doc-toggle-chevron-color: var(--text-muted);
    --doc-divider-color: var(--border);
    /* Фон-сетка свободной раскладки: заметна, но не спорит с текстом. */
    --doc-grid-color: var(--border-soft);
    /* Габариты блока: заливка под курсором и рамка текущего. */
    --doc-block-hover-color: var(--surface-hover);
    --doc-block-selected-color: var(--primary);
    --doc-media-bg: var(--surface-hover);
    --doc-embed-border-color: var(--border);
    --doc-embed-bg: var(--surface-hover);
    --doc-table-border-color: var(--border);
    --doc-table-header-bg: var(--surface-hover);
    --doc-menu-bg: var(--bg-panel);
    --doc-menu-border: var(--border);
    --doc-menu-sel-bg: var(--surface-hover);
    /* Слева нужно место под ручку ⋮⋮ (16px + зазор 6px), иначе её
     * прижимает к краю панели и она налезает на текст. */
    --doc-padding: 28px;
}

/* Настройки раскладки страницы в «Свойствах». */
.notes-props-row {
    min-height: 26px;
}
/* Дерево блоков: те же строки, что у дерева страниц, плюс тип и метка
 * закрепления на холсте. */
.notes-block-row {
    padding: 0px 6px;
}
.notes-block-icon {
    icon-size: 14px;
    color: var(--text-subtle);
}
.notes-block-pin {
    icon-size: 12px;
    color: var(--primary);
}

/* Переключатель из иконок в свойствах блока (выравнивание, начертание). */
.notes-props-seg {
    width: 26px;
    height: 24px;
    border-radius: 6px;
    background-color: var(--bg-search);
    transition: background-color 120ms ease-out;
}
.notes-props-seg:hover {
    background-color: var(--surface-hover);
}
.notes-props-seg.selected {
    background-color: var(--primary);
}
.notes-props-seg-icon {
    icon-size: 15px;
    color: var(--text-muted);
}
.notes-props-seg.selected .notes-props-seg-icon {
    color: var(--on-primary);
}
.notes-props-swatch.selected {
    border-width: 2px;
    border-color: var(--primary);
}

/* Поля раскладки: компактнее дефолтных 40px, иначе панель распухает. */
.notes-props-field {
    height: 28px;
    font-size: 12px;
    border-radius: 8px;
}
.notes-props-row-label {
    font-size: 12px;
    color: var(--text-muted);
}
.notes-props-hint {
    font-size: 11px;
    color: var(--text-subtle);
}

/* ── Правая панель ────────────────────────────────────────────── */
.notes-link-row {
    border-radius: 6px;
    padding: 6px 8px;
    transition: background-color 120ms ease-out;
}
.notes-link-row:hover {
    background-color: var(--surface-hover);
}
.notes-link-icon {
    icon-size: 16px;
    font-size: 14px;
    color: var(--text-muted);
}
.notes-link-icon.emoji {
    color: var(--text);
}
.notes-link-label {
    font-size: 13px;
    color: var(--text);
}
.notes-props-icon-box {
    width: 30px;
    height: 30px;
    border-radius: 6px;
    background-color: var(--surface-hover);
}
.notes-props-icon-box:hover {
    background-color: var(--surface-selected);
}
.notes-props-icon {
    icon-size: 18px;
    font-size: 16px;
    color: var(--text-muted);
}
.notes-props-icon.emoji {
    color: var(--text);
}

/* ── Панель выбора иконки ─────────────────────────────────────── */
.notes-icon-picker {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border);
    border-radius: 12px;
    padding: 8px;
}
.notes-icon-picker-header {
    padding: 0 0 6px 0;
}
.notes-icon-tab {
    padding: 4px 10px;
    border-radius: 6px;
}
.notes-icon-tab:hover {
    background-color: var(--surface-hover);
}
.notes-icon-tab.selected {
    background-color: var(--surface-selected);
}
.notes-icon-tab-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
}
.notes-icon-grid {
    padding: 2px;
}
.notes-icon-section {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    padding: 6px 2px 2px 2px;
}
.notes-icon-cell {
    width: 34px;
    height: 32px;
    border-radius: 6px;
}
.notes-icon-cell:hover {
    background-color: var(--surface-hover);
}
.notes-icon-glyph {
    icon-size: 20px;
    font-size: 18px;
    color: var(--text);
}

/* Врезки: шапка и рамка. */
.notes-embed-icon {
    icon-size: 16px;
    color: var(--text-muted);
}

/* ── Плитка рейла ─────────────────────────────────────────────── */
.nav-rail-note-tile {
    width: 40px;
    height: 40px;
    border-radius: 12px;
    background-color: var(--surface-hover);
    border-width: 1px;
    border-color: transparent;
    transition: border-color 120ms ease-out;
}
.nav-rail-note-tile:hover {
    border-color: var(--border-strong);
}
.nav-rail-note-tile.selected {
    border-color: var(--primary);
}
.nav-rail-note-icon {
    font-size: 20px;
    color: var(--text);
}

/* ── Конфликт внешнего изменения ──────────────────────────────── */
.notes-conflict-banner {
    background-color: var(--warning-soft);
    border-width: 1px;
    border-color: var(--warning);
    border-radius: 8px;
    padding: 8px 12px;
    margin: 8px 12px 0px 12px;
}
.notes-conflict-icon {
    font-size: 18px;
    color: var(--warning);
}
.notes-conflict-text {
    font-size: 13px;
    color: var(--text);
}
.notes-conflict-btn {
    font-size: 12px;
}

/* ── Вкладка «Связи» ──────────────────────────────────────────── */
.notes-links-list {
    padding: 8px;
}
.notes-links-section {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-subtle);
    padding: 8px 4px 2px 4px;
}

/* ── Базы данных ──────────────────────────────────────────────── */
.notes-base-switcher {
    padding: 8px 12px 4px 12px;
}
.notes-base-tab {
    border-radius: 6px;
    padding: 4px 10px;
    transition: background-color 120ms ease-out;
}
.notes-base-tab:hover {
    background-color: var(--surface-hover);
}
.notes-base-tab.selected {
    background-color: var(--surface-selected);
}
.notes-base-tab-icon {
    font-size: 14px;
    color: var(--text-muted);
}
.notes-base-tab-label {
    font-size: 12px;
    color: var(--text);
}
.notes-base-table {
    margin: 4px 12px;
}
.notes-base-add-row {
    border-radius: 6px;
    padding: 6px 12px;
    margin: 0px 12px 10px 12px;
}
.notes-base-add-row:hover {
    background-color: var(--surface-hover);
}
.notes-base-row-delete {
    font-size: 13px;
}

/* ── Канбан ───────────────────────────────────────────────────── */
.notes-kanban {
    padding: 10px 12px;
}
.notes-kanban-lane {
    width: 260px;
    border-radius: 10px;
    background-color: var(--surface-hover);
    padding: 8px;
}
.notes-kanban-lane-header {
    padding: 2px 4px 6px 4px;
}
.notes-kanban-dot {
    width: 9px;
    height: 9px;
    border-radius: 5px;
}
.notes-kanban-lane-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
}
.notes-kanban-lane-count {
    font-size: 11px;
    color: var(--text-subtle);
}
.notes-kanban-card {
    border-radius: 8px;
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 8px 10px;
}
.notes-kanban-card-title {
    font-size: 13px;
    color: var(--text);
}
.notes-kanban-card-meta {
    font-size: 11px;
    color: var(--text-muted);
}

/* ── Канвасы ──────────────────────────────────────────────────── */
.notes-canvas-viewport {
    background-color: var(--bg-chat);
}
.notes-canvas-toolbar {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    border-radius: 10px;
    padding: 4px;
}
.notes-canvas-card {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border);
    border-radius: 10px;
}
.notes-canvas-card.selected {
    border-color: var(--primary);
    border-width: 2px;
}

/* ── Врезки ![[…]] ────────────────────────────────────────────── */
.notes-embed-header {
    padding: 2px 4px;
}
.notes-embed-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
}
.notes-embed-frame {
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border-soft);
    background-color: var(--bg-panel);
}

/* ── Граф связей ──────────────────────────────────────────────── */
.notes-graph-viewport {
    background-color: var(--bg-chat);
}
.notes-mini-graph {
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border-soft);
    background-color: var(--bg-panel);
    margin: 0px 0px 6px 0px;
}

/* ── Свойства ─────────────────────────────────────────────────── */
.notes-props {
    padding: 10px;
}
.notes-props-name {
    font-size: 13px;
}
.notes-props-path {
    font-size: 11px;
    color: var(--text-muted);
}
.notes-props-swatch {
    width: 22px;
    height: 22px;
    border-radius: 11px;
    border-width: 1px;
    border-color: var(--border-strong);
}
.notes-props-swatch.empty {
    background-color: var(--surface-hover);
}
.notes-props-delete {
    font-size: 12px;
}
