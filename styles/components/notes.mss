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
/* Зоны дропа «перед / внутрь / после» считаются от высоты цели
 * (contents.rs::zone_at), над каждой строкой — линия вставки. */
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
/* Курсор с переносом над серединой строки — «вложить внутрь». */
.notes-tree-row.drop-into {
    background-color: var(--surface-selected);
    outline-width: 1px;
    outline-color: var(--primary);
    outline-offset: -1px;
}
/* Линия вставки над строкой: всегда 2px, красится только под курсором. */
.notes-tree-drop-line {
    background-color: transparent;
    border-radius: 1px;
}
.notes-tree-drop-line.active {
    background-color: var(--primary);
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
    /* Примитивы: контур по умолчанию — приглушённый текст, хваталки
     * концов и углов — акцентом, чтобы их было видно на любой фигуре. */
    --doc-shape-stroke-color: var(--text-muted);
    --doc-shape-handle-color: var(--primary);
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

/* Кнопки истории правок (отменить/повторить) в шапке центра — компактные,
 * прозрачные; недоступную ToolButton рисует полупрозрачной сам. */
.notes-history-btn {
    width: 28px;
    height: 28px;
    icon-size: 18px;
    border-radius: 8px;
    background-color: transparent;
    color: var(--text-subtle);
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}
.notes-history-btn:hover {
    background-color: var(--surface-hover);
    color: var(--text);
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
/* Точный цвет: ColorPicker берёт цвета попапа из своего MSS, а без класса
 * падал на светлые умолчания виджета (белая палитра в тёмной теме) и на
 * высоту 40px рядом с 28-пиксельными соседями. */
.notes-props-color {
    height: 28px;
    font-size: 11px;
    border-radius: 8px;
    background-color: var(--bg-panel);
    color: var(--text);
    border-color: var(--border);
    accent-color: var(--primary);
    --popup-background: var(--bg-shell);
    --popup-color: var(--text);
    --popup-border: var(--border);
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

/* ── Канбан-доска ─────────────────────────────────────────────── */
.notes-kanban {
    padding: 10px 12px;
}
.notes-kanban-lane {
    border-radius: 12px;
    background-color: var(--surface-hover);
    padding: 8px;
}
.notes-kanban-lane-header {
    padding: 2px 4px 6px 4px;
}
.notes-kanban-dot {
    width: 10px;
    height: 10px;
    border-radius: 5px;
}
.notes-kanban-dot.empty {
    border-width: 1px;
    border-color: var(--border-strong);
}
.notes-kanban-lane-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    background-color: transparent;
    border-width: 0px;
    padding: 2px 4px;
}
.notes-kanban-lane-count {
    font-size: 11px;
    color: var(--text-subtle);
}
.notes-kanban-lane-btn {
    font-size: 14px;
}
/* «В календарь»: у запланированной карточки кнопка акцентная — видно, что
 * полоса уже стоит, не открывая меню. */
.notes-kanban-lane-btn.selected {
    color: var(--primary);
}
/* Кромка колонки: узкая, подсвечивается под курсором. */
.notes-kanban-resize {
    width: 6px;
    border-radius: 3px;
}
.notes-kanban-resize:hover {
    background-color: var(--border-strong);
}
/* Карточка: цветная полоса колонки слева, заголовок, markdown-контент. */
.notes-kanban-card {
    border-radius: 10px;
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 10px 12px 10px 8px;
}
.notes-kanban-card:hover {
    border-color: var(--border-strong);
}
.notes-kanban-card.editing {
    border-color: var(--primary);
}
.notes-kanban-card.selected {
    border-color: var(--primary);
}
/* Плейсхолдер места вставки при переносе: пустая карточка высотой с
   переносимую; неактивный — нулевой высоты. */
.notes-kanban-gap {
    border-radius: 10px;
}
.notes-kanban-gap.active {
    border-width: 1px;
    border-color: var(--primary);
    background-color: var(--surface-selected);
}
/* Чипы приоритета и меток; цвет — из данных (`model::chip_colors`: фон —
 * цвет метки, текст — чёрный/белый по яркости), здесь только форма.
 * Ширину чипа оценивает `view::chip_width` — менять padding вместе с ней. */
.notes-kanban-chip {
    border-radius: 6px;
    padding: 2px 8px;
}
.notes-kanban-chip-text {
    font-size: 10px;
    font-weight: 600;
}
.notes-kanban-chip-more {
    font-size: 10px;
    color: var(--text-subtle);
}
.notes-kanban-card-preview {
    font-size: 12px;
    color: var(--text-muted);
}
.notes-kanban-due {
    border-radius: 6px;
    padding: 1px 6px 1px 4px;
    background-color: var(--surface-hover);
}
.notes-kanban-due.overdue {
    background-color: var(--primary-soft);
}
.notes-kanban-due-icon {
    font-size: 13px;
    color: var(--text-muted);
}
.notes-kanban-due.overdue .notes-kanban-due-icon {
    color: var(--error);
}
.notes-kanban-due-text {
    font-size: 11px;
    color: var(--text-muted);
}
.notes-kanban-due.overdue .notes-kanban-due-text {
    color: var(--error);
    font-weight: 600;
}
.notes-kanban-progress-text {
    font-size: 11px;
    color: var(--text-muted);
}
.notes-kanban-progress {
    width: 56px;
}
.notes-kanban-progress-bar {
    height: 4px;
    border-radius: 2px;
}
/* Поля выбранной карточки под текстом. */
.notes-kanban-card-fields {
    padding: 4px 0px 0px 0px;
}
.notes-kanban-field {
    font-size: 11px;
    padding: 4px 8px;
    height: 26px;
    border-radius: 7px;
}
.notes-props-card-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    padding: 0px 4px;
}
.notes-kanban-card-accent {
    width: 3px;
    border-radius: 2px;
}
.notes-kanban-card-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}
.notes-kanban-card-body {
    font-size: 12px;
    color: var(--text-muted);
}
.notes-kanban-card-divider {
    height: 1px;
    background-color: var(--border-soft);
}
/* Редактор карточки: заголовок — блок `## …`, поэтому h2 ужат под карточку. */
.notes-kanban-card-editor {
    --doc-padding: 4px;
    --doc-text-size: 12px;
    --doc-h2-size: 15px;
    --doc-h1-size: 16px;
    --doc-h3-size: 14px;
}
.notes-kanban-card-toolbar {
    padding: 2px 0px 0px 0px;
}
.notes-kanban-tail {
    border-radius: 6px;
    padding: 4px 8px;
    margin-top: 6px;
}
.notes-kanban-tail:hover {
    background-color: var(--surface-selected);
}
.notes-kanban-tail-icon {
    font-size: 18px;
    color: var(--text-subtle);
}
.notes-kanban-tail:hover .notes-kanban-tail-icon {
    color: var(--text);
}

/* ── Диаграмма Ганта ──────────────────────────────────────────── */
.notes-gantt-toolbar {
    padding: 6px 8px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}
.notes-gantt-labels {
    border-right-width: 1px;
    border-color: var(--border-soft);
}
.notes-gantt-labels-header {
    padding: 0px 12px;
}
.notes-gantt-labels-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-subtle);
}
.notes-gantt-row {
    padding: 0px 6px 0px 10px;
}
.notes-gantt-task-name {
    font-size: 13px;
    color: var(--text);
    background-color: transparent;
    border-width: 0px;
    padding: 2px 4px;
}
/* Цвета шкалы: текст, сетка, акцент (бар без своего цвета, «сегодня»). */
.notes-gantt-chart {
    color: var(--text-muted);
    border-color: var(--border-soft);
    accent-color: var(--primary);
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

/* ── Интеллект-карта ──────────────────────────────────────────── */
.notes-mindmap-toolbar {
    padding: 6px 8px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}
.notes-mindmap-direction {
    font-size: 12px;
}
.notes-mindmap-viewport {
    background-color: var(--bg-chat);
}
/* Цвета холста: текст, приглушённые линии/обводки, акцент (корень,
 * выбор), фон без своего цвета в стиле карты. */
.notes-mindmap-canvas {
    color: var(--text);
    border-color: var(--text-muted);
    accent-color: var(--primary);
    background-color: var(--bg-chat);
}
.notes-mindmap-node-editor {
    font-size: 13px;
    background-color: var(--bg-search);
    color: var(--text);
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--primary);
    padding: 6px 8px;
}
.notes-mindmap-note {
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border-soft);
    background-color: var(--bg-search);
    padding: 4px 6px;
    height: 110px;
}
.notes-mindmap-note-editor {
    font-size: 12px;
}

/* ── Календарь ────────────────────────────────────────────────── */
.notes-calendar-toolbar {
    padding: 6px 8px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}
.notes-calendar-view-btn.selected {
    background-color: var(--surface-hover);
    color: var(--primary);
}
.notes-calendar-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    padding: 0px 6px;
}
.notes-calendar-scroll {
    background-color: var(--bg-chat);
}
/* Цвета сеток: текст, линии, акцент («сегодня», выбор), фон. */
.notes-calendar-grid {
    color: var(--text);
    border-color: var(--border-soft);
    accent-color: var(--primary);
    background-color: var(--bg-chat);
}
/* Отступ панели: до 09.09.2026 PopupPanel игнорировал padding из MSS, и
 * поля попапа касались краёв окна (см. popup_panel.rs::padding). */
.notes-calendar-popup {
    background-color: var(--bg-shell);
    border-radius: 12px;
    border-width: 1px;
    border-color: var(--border);
    padding: 14px;
}
.notes-calendar-popup-title {
    font-size: 14px;
    font-weight: 600;
    background-color: var(--bg-search);
    color: var(--text);
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 8px 10px;
}
.notes-calendar-popup-label {
    font-size: 12px;
    color: var(--text-muted);
}
.notes-calendar-note {
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border-soft);
    background-color: var(--bg-search);
    padding: 4px 6px;
    height: 80px;
}
.notes-calendar-note-editor {
    font-size: 12px;
}
.notes-calendar-chip {
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 2px 8px;
    color: var(--text-muted);
}
.notes-calendar-chip.selected {
    background-color: var(--surface-hover);
    color: var(--text);
}
.notes-calendar-chip-text {
    font-size: 11px;
}
/* Дни недели повтора в попапе события: семь узких чипов в одну строку. */
.notes-calendar-day-chip {
    width: 28px;
    height: 22px;
    border-radius: 6px;
    border-width: 1px;
    border-color: var(--border-soft);
    color: var(--text-muted);
}
.notes-calendar-day-chip.selected {
    background-color: var(--primary);
    border-color: var(--primary);
    color: var(--on-primary);
}
/* Список дня («ещё n»): строки с маркером цвета, временем и названием. */
.notes-calendar-day-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}
.notes-calendar-day-row {
    padding: 4px 6px;
    border-radius: 6px;
}
.notes-calendar-day-row:hover {
    background-color: var(--surface-hover);
}
.notes-calendar-day-marker {
    width: 10px;
    height: 10px;
    border-radius: 5px;
}
.notes-calendar-day-marker.task {
    width: 4px;
    height: 14px;
    border-radius: 2px;
}
.notes-calendar-day-time {
    font-size: 11px;
    color: var(--text-muted);
    width: 78px;
}
.notes-calendar-day-name {
    font-size: 12px;
    color: var(--text);
}
.notes-calendar-day-name.done {
    color: var(--text-muted);
    text-decoration: line-through;
}

/* ── Графики (виджеты syngui) ──────────────────────────────────────
 * `width/height: 100%` — график занимает свою врезку целиком: без этого
 * он берёт встроенные умолчания библиотеки (600×400 у линий, 300×300 у
 * круговой) и висит куском в углу блока. Цвета осей, подписей и заголовка
 * графики выводят из `color`, поэтому одной строки хватает на всю тему;
 * `background-color` радару нужен явно — его умолчание белое. */
.notes-chart {
    width: 100%;
    height: 100%;
    color: var(--text);
    background-color: transparent;
    grid-color: var(--border-soft);
    axis-color: var(--text-subtle);
    label-color: var(--text-muted);
    track-color: var(--surface-hover);
    needle-color: var(--text);
    tooltip-background: var(--bg-panel);
    tooltip-border-color: var(--border-soft);
    axis-font-size: 11px;
    label-font-size: 11px;
    legend-font-size: 12px;
    title-font-size: 14px;
    value-font-size: 22px;
}

/* Штампы карточки, повтор, закрытая карточка. */
.notes-kanban-due.done {
    background-color: var(--surface-hover);
}
.notes-kanban-due.done .notes-kanban-due-icon {
    color: var(--success);
}
.notes-kanban-repeat-icon {
    font-size: 13px;
    color: var(--text-subtle);
}
.notes-kanban-dates {
    font-size: 10px;
    color: var(--text-subtle);
}
/* Таймеры колонок: обратный отсчёт на карточке и значок в шапке колонки. */
.notes-kanban-due.timer.soon {
    background-color: var(--primary-soft);
}
.notes-kanban-due.timer.soon .notes-kanban-due-icon {
    color: var(--warning);
}
.notes-kanban-due.timer.soon .notes-kanban-due-text {
    color: var(--warning);
    font-weight: 600;
}
.notes-kanban-lane-timer {
    font-size: 14px;
    color: var(--text-subtle);
}
/* Флаг «готово» у колонки в панели свойств. */
.notes-props-col-done {
    width: 24px;
    height: 24px;
    icon-size: 16px;
    border-radius: 6px;
    background-color: transparent;
    color: var(--text-subtle);
}
.notes-props-col-done.on {
    color: var(--success);
    background-color: var(--surface-selected);
}
/* Напоминания: колокольчик в шапке и попап. */
.notes-reminders-bell {
    width: 28px;
    height: 28px;
    icon-size: 18px;
    border-radius: 8px;
    background-color: transparent;
    color: var(--text-muted);
}
.notes-reminders-bell:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}
.notes-reminders-bell.hot {
    color: var(--primary);
}
.notes-reminders-badge {
    font-size: 11px;
    font-weight: 600;
    color: var(--primary);
}
.notes-reminders-popup {
    padding: 10px 12px;
    border-radius: 12px;
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border);
}
.notes-reminders-heading {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}
.notes-reminders-journal {
    height: 26px;
    font-size: 11px;
    padding: 0px 8px;
    border-radius: 7px;
}
.notes-reminders-empty {
    font-size: 12px;
    color: var(--text-muted);
    padding: 6px 4px;
}
.notes-reminders-row {
    padding: 5px 6px;
    border-radius: 8px;
}
.notes-reminders-row:hover {
    background-color: var(--surface-hover);
}
.notes-reminders-row-icon {
    font-size: 16px;
    color: var(--text-muted);
}
.notes-reminders-row.overdue .notes-reminders-row-icon {
    color: var(--error);
}
.notes-reminders-row-title {
    font-size: 12px;
    color: var(--text);
}
.notes-reminders-row-sub {
    font-size: 11px;
    color: var(--text-subtle);
}
/* Вложения карточки: миниатюры и файлы-чипы; окно просмотра картинки. */
.notes-kanban-thumb {
    width: 64px;
    height: 64px;
    border-radius: 8px;
    background-color: var(--surface-hover);
}
.notes-kanban-thumb-img {
    width: 64px;
    height: 64px;
}
.notes-kanban-file {
    border-radius: 6px;
    padding: 2px 8px 2px 6px;
    background-color: var(--surface-hover);
}
.notes-kanban-file:hover {
    background-color: var(--surface-selected);
}
.notes-kanban-file-icon {
    font-size: 14px;
    color: var(--text-muted);
}
.notes-kanban-file-text {
    font-size: 11px;
    color: var(--text-muted);
}
/* Окно просмотра картинки-вложения карточки (`media::image_viewer`).
 * Палитра темы, как у остальных плавающих окон (`.chat-float-window`):
 * с жёстко тёмной «галерейной» палитрой окно на светлой теме выглядело
 * чужим. Сцена — `--bg-window`: на тон отличается от панели, так что края
 * картинки со светлым фоном видны в обеих темах.
 * Шапку FloatingWindow рисует сам: фон окна с затемнением, заголовок и
 * крестик — `color`. padding 1px — сцена от края до края, но не поверх
 * рамки; у подвала своего фона нет, иначе его квадратные углы вылезли бы
 * за скругление окна. */
.notes-image-viewer-window {
    background-color: var(--bg-panel);
    color: var(--text);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-panel);
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.18), var(--glass-shadow);
    padding: 1px 1px 1px 1px;
    min-width: 360px;
    min-height: 280px;
}
.notes-image-viewer {
    width: 100%;
    height: 100%;
}
.notes-image-viewer-stage {
    background-color: var(--bg-window);
    flex-grow: 1;
    overflow: hidden;
}
.notes-image-viewer-img {
    width: 100%;
    height: 100%;
}
.notes-image-viewer-footer {
    padding: 6px 10px 6px 10px;
    border-width: 0;
    border-top-width: 1px;
    border-color: var(--border-soft);
}
/* Кнопки подвала: цвет и наведение — из ресета ToolButton (reset.mss),
 * здесь только размер. Классы чата (`media-viewer-*`) не годятся: там
 * белые глифы под его всегда тёмный просмотрщик. */
.notes-image-viewer-action {
    width: 32px;
    height: 32px;
    icon-size: 18px;
}
.notes-image-viewer-zoom {
    color: var(--text-muted);
    font-size: 12px;
    width: 48px;
    text-align: center;
}
