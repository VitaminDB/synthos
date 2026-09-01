/* Режим «Заметки»: дерево vault, редактор, правая панель, плитка рейла. */

/* Разделители трёхпанельного каркаса. */
.notes-h-split {
    background-color: transparent;
}
.notes-h-split:hover {
    background-color: var(--primary-soft);
}

/* ── Дерево vault ─────────────────────────────────────────────── */
.notes-tree {
    padding: 6px;
}
.notes-tree-row {
    border-radius: 6px;
    padding: 4px 6px;
    transition: background-color 120ms ease-out;
}
.notes-tree-row:hover {
    background-color: var(--surface-hover);
}
.notes-tree-row.selected {
    background-color: var(--surface-selected);
}
.notes-tree-icon {
    font-size: 16px;
    color: var(--text-muted);
}
.notes-tree-name {
    font-size: 13px;
    color: var(--text);
}
.notes-tree-empty {
    padding: 16px 10px;
}
.notes-tree-empty-text {
    font-size: 12px;
    color: var(--text-muted);
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
    --doc-media-bg: var(--surface-hover);
    --doc-embed-border-color: var(--border);
    --doc-embed-bg: var(--surface-hover);
    --doc-table-border-color: var(--border);
    --doc-table-header-bg: var(--surface-hover);
    --doc-menu-bg: var(--bg-panel);
    --doc-menu-border: var(--border);
    --doc-menu-sel-bg: var(--surface-hover);
}

/* ── Правая панель ────────────────────────────────────────────── */
.notes-insert-list {
    padding: 8px;
}
.notes-insert-hint {
    font-size: 12px;
    color: var(--text-muted);
    padding: 2px 4px 8px 4px;
}
.notes-insert-row {
    border-radius: 6px;
    padding: 6px 8px;
    transition: background-color 120ms ease-out;
}
.notes-insert-row:hover {
    background-color: var(--surface-hover);
}
.notes-insert-icon {
    font-size: 16px;
    color: var(--text-muted);
}
.notes-insert-label {
    font-size: 13px;
    color: var(--text);
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
