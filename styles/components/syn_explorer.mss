/* ─────────────────────── SynExplorer page ───────────────────────
 * Страница работы с `.syn` model-bundle пакетами.
 *
 * Все цвета/радиусы/тайминги — через токены `styles/base/variables.mss`.
 * Локальные переменные в `:root` ниже — производные от базовых; держим их
 * рядом со страницей, чтобы держать дизайн-альбом этой фичи в одном месте
 * (как сделано в `code_editor.mss` с `--file-color-*`).
 * ────────────────────────────────────────────────────────────────── */

:root {
    /* Локальные поверхности SynExplorer — производные от --bg-panel и
     * --surface-hover. Используем именно их (а не дополнительные оттенки)
     * чтобы страница визуально жила в той же палитре, что и остальной synthos. */
    --syn-card-bg:        var(--bg-search);
    --syn-card-bg-hover:  var(--surface-hover);
    --syn-card-bg-active: var(--primary-soft);
    --syn-divider:        var(--border-soft);
    --syn-input-bg:       var(--bg-search);

    /* Радиусы. Базовая палитра содержит только размер-специфичные
     * (`--radius-panel` 16px, `--radius-bubble` 18px); для маленьких элементов
     * вводим именованный токен. */
    --syn-radius-card:    var(--radius-panel);
    --syn-radius-pill:    var(--radius-pill);
    --syn-radius-input:   10px;
    --syn-radius-tile:    8px;
}

.syn-explorer-page {
    background-color: var(--bg-shell);
}

.syn-explorer-body {
    background-color: var(--bg-shell);
}

/* SplitView дивайдеры — единый стиль с `.code-editor-h-split`. */
.syn-explorer-h-split {
    background-color: var(--bg-shell);
    border-color: var(--syn-divider);
    accent-color: var(--primary);
    divider-thickness: 1px;
}

/* ─── Tooltip (глобально, element-selector по типу `Tooltip`) ────
 * Цвета — дефолтные из syngui (`tooltip.rs`): bg `#1E1F22`, border `#3F4147`,
 * текст `#FFFFFF`. Прописываем их явно в MSS, потому что добавляем
 * padding/border-radius/font-size: без явных color-полей рендерер взял бы
 * fallback'и из MSS селектора `*` (если бы они были) и стиль «уплыл». */
Tooltip {
    padding: 10px 14px 10px 14px;
    background-color: #1E1F22;
    color: #FFFFFF;
    border-color: #3F4147;
    border-radius: 10px;
    font-size: 12px;
    max-width: 320px;
}

/* ─── Toolbar ───────────────────────────────────────────────────── */

/* Кнопки бывшего toolbar'а — правый кластер общей шапки. */
.syn-explorer-actions {
}

.syn-toolbar-btn {
    width: 36px;
    height: 36px;
    background-color: transparent;
    border-radius: var(--syn-radius-tile);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.syn-toolbar-btn:hover {
    background-color: var(--surface-hover);
}

.syn-toolbar-btn.primary {
    background-color: var(--primary);
    color: var(--on-primary);
}

.syn-toolbar-btn.primary:hover {
    background-color: var(--primary-hover);
}

.syn-toolbar-btn.danger {
    color: var(--error);
}

.syn-toolbar-btn.danger:hover {
    background-color: var(--primary-soft);
}

.syn-toolbar-btn.disabled {
    color: var(--text-subtle);
    background-color: transparent;
}

.syn-toolbar-divider {
    width: 1px;
    height: 24px;
    background-color: var(--border);
    margin: 0 6px 0 6px;
}


/* Заголовки боковых панелей («Закладки +», «Файлы пакета N ⤓») теперь
 * живут в `.panel-header` каркаса — обёртка без своих отступов и фона. */
.syn-panel-header-row {
    background-color: transparent;
}

/* ─── Left panel ────────────────────────────────────────────────── */

.syn-explorer-left-panel {
    background-color: var(--bg-panel);
    border-right-width: 1px;
    border-color: var(--syn-divider);
}

.syn-bookmark-section-header {
    height: 40px;
    padding: 10px 16px 10px 16px;
    background-color: var(--bg-panel);
    border-bottom-width: 1px;
    border-color: var(--syn-divider);
}

.syn-section-title {
    color: var(--text);
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.5px;
}

.syn-section-title-icon {
    color: var(--text-muted);
    font-size: 16px;
}

.syn-section-count {
    color: var(--text-subtle);
    font-size: 11px;
    background-color: var(--surface-hover);
    border-radius: var(--syn-radius-pill);
    padding: 2px 8px 2px 8px;
}

.syn-spacer {
    background-color: transparent;
}

.syn-icon-btn {
    width: 28px;
    height: 28px;
    background-color: transparent;
    border-radius: var(--syn-radius-tile);
    color: var(--text-muted);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.syn-icon-btn:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.syn-bookmarks-list {
    padding: 6px 8px 10px 8px;
    background-color: transparent;
}

.syn-bookmarks-empty {
    padding: 14px 16px 14px 16px;
    background-color: transparent;
}

.syn-bookmark-item {
    padding: 8px 12px 8px 12px;
    border-radius: var(--syn-radius-tile);
    background-color: transparent;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.syn-bookmark-item:hover {
    background-color: var(--surface-hover);
}

.syn-bookmark-item.selected {
    background-color: var(--primary-soft);
}

.syn-bookmark-icon {
    color: var(--primary);
    font-size: 18px;
}

.syn-bookmark-label {
    color: var(--text);
    font-size: 13px;
}

.syn-visually-hidden {
    color: transparent;
    font-size: 1px;
    max-lines: 1;
}

.syn-folder-section {
    background-color: transparent;
}

.syn-folder-body {
    padding: 8px 8px 12px 8px;
    background-color: transparent;
}

.syn-folder-placeholder {
    padding: 20px 16px 20px 16px;
    background-color: transparent;
}

.syn-empty-hint {
    color: var(--text-muted);
    font-size: 12px;
}

.syn-placeholder-icon-mini {
    color: var(--text-subtle);
    font-size: 28px;
}

/* Карточка `.syn` файла */
.syn-bundle-card {
    padding: 12px 14px 12px 14px;
    border-radius: var(--syn-radius-card);
    background-color: var(--syn-card-bg);
    border-width: 1px;
    border-color: transparent;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.syn-bundle-card:hover {
    background-color: var(--syn-card-bg-hover);
}

.syn-bundle-card.selected {
    background-color: var(--syn-card-bg-active);
    border-color: var(--primary);
}

.syn-bundle-card-icon {
    color: var(--primary);
    font-size: 24px;
}

.syn-bundle-card-name {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
}

.syn-bundle-card-meta {
    color: var(--text-muted);
    font-size: 11px;
}

/* ─── Center ────────────────────────────────────────────────────── */

.syn-explorer-center {
    background-color: var(--bg-shell);
}

.syn-tab-bar {
    height: 44px;
    padding: 0 16px 0 16px;
    background-color: var(--bg-panel);
    border-bottom-width: 1px;
    border-color: var(--syn-divider);
}

.syn-tab {
    padding: 8px 14px 8px 14px;
    margin-right: 4px;
    border-radius: var(--syn-radius-tile);
    background-color: transparent;
    border-bottom-width: 2px;
    border-color: transparent;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.syn-tab:hover {
    background-color: var(--surface-hover);
}

.syn-tab.active {
    background-color: var(--primary-soft);
    border-color: var(--primary);
}

.syn-tab-icon {
    color: var(--text-muted);
    font-size: 16px;
}

.syn-tab.active .syn-tab-icon {
    color: var(--primary);
}

.syn-tab-label {
    color: var(--text);
    font-size: 13px;
}

.syn-tab.active .syn-tab-label {
    color: var(--primary);
    font-weight: 600;
}

.syn-tab-content {
    background-color: var(--bg-shell);
    transition: opacity var(--duration-fast) var(--ease-standard);
}

.syn-tab-pane {
    padding: 18px 20px 18px 20px;
    background-color: var(--bg-shell);
}

/* ─── Overview tab ──────────────────────────────────────────────── */

.syn-overview-card {
    padding: 16px 18px 16px 18px;
    border-radius: var(--syn-radius-card);
    background-color: var(--syn-card-bg);
    border-width: 1px;
    border-color: var(--syn-divider);
}

.syn-overview-title {
    color: var(--text);
    font-size: 20px;
    font-weight: 700;
}

.syn-overview-subtitle {
    color: var(--text);
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.3px;
}

.syn-overview-line {
    color: var(--text-muted);
    font-size: 13px;
}

.syn-stat-chip {
    padding: 10px 16px 10px 16px;
    border-radius: var(--syn-radius-card);
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--syn-divider);
    min-width: 88px;
}

.syn-stat-value {
    color: var(--text);
    font-size: 20px;
    font-weight: 700;
}

.syn-stat-label {
    color: var(--text-muted);
    font-size: 11px;
    letter-spacing: 0.3px;
}

/* ─── Files tab (TableView) ─────────────────────────────────────── */

/* MSS-свойства TableView (см. syngui/widgets/data/table_view/element.rs:1455+):
 *  header-bg / header-color / header-padding / row-hover-bg /
 *  row-selected-bg / row-striped-bg / row-padding / cell-padding /
 *  cell-font-size / grid-alpha.
 * `color` (текст ячеек) и `border-color` (grid-lines) без явного MSS-fallback
 * имеют хардкод-defaults в TableView (`#334155` / `#E2E8F0`), невидимые на
 * тёмной теме — обязательно прописываем токены приложения. */
.syn-files-table {
    background-color: var(--bg-panel);
    color: var(--text);
    border-radius: var(--syn-radius-card);
    border-width: 1px;
    border-color: var(--syn-divider);
    accent-color: var(--primary);
    header-bg: var(--bg-search);
    header-color: var(--text);
    header-font-size: 12px;
    header-padding: 12px;
    row-padding: 10px;
    row-hover-bg: var(--surface-hover);
    row-selected-bg: var(--primary-soft);
    row-striped-bg: var(--bg-search);
    cell-padding: 10px;
    cell-font-size: 13px;
    grid-alpha: 0.4px;
}

/* ─── Metadata tab ──────────────────────────────────────────────── */

.syn-property-label {
    color: var(--text);
    font-size: 13px;
    font-weight: 700;
}

.syn-property-hint {
    color: var(--text-muted);
    font-size: 11px;
}

.syn-meta-input {
    background-color: var(--syn-input-bg);
    border-radius: var(--syn-radius-input);
    border-width: 1px;
    border-color: var(--syn-divider);
    padding: 8px 12px 8px 12px;
}

.syn-meta-readonly-block {
    padding: 12px 14px 12px 14px;
    border-radius: var(--syn-radius-card);
    background-color: var(--syn-card-bg);
    border-width: 1px;
    border-color: var(--syn-divider);
}

.syn-meta-readonly-text {
    color: var(--text-muted);
    font-size: 12px;
    font-family: monospace;
}

.syn-meta-hint {
    color: var(--text-subtle);
    font-size: 11px;
}

/* ─── Preview tab ───────────────────────────────────────────────── */

.syn-preview-frame {
    padding: 0;
    border-radius: var(--syn-radius-card);
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--syn-divider);
    /* CodeEditor / Image / hex-dump имеют собственный фон и scrollbar — без
     * clipping содержимое выпирает за скруглённые углы рамки. */
    overflow: hidden;
}

.syn-preview-image {
    padding: 12px 12px 12px 12px;
}

.syn-preview-text {
    padding: 0;
}

.syn-preview-codeeditor {
    background-color: var(--bg-panel);
}

.syn-preview-hex {
    padding: 14px 16px 14px 16px;
}

.syn-preview-hex-header {
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 700;
}

.syn-preview-hex-body {
    color: var(--text);
    font-size: 12px;
    font-family: monospace;
}

/* ─── Right panel ───────────────────────────────────────────────── */

.syn-explorer-right-panel {
    background-color: var(--bg-panel);
    border-left-width: 1px;
    border-color: var(--syn-divider);
}

.syn-explorer-right-body {
    padding: 6px 0 0 0;
    background-color: transparent;
}

/* TreeView без явного `color`/`accent-color` использует хардкод-defaults
 * (`#1F2937` / `#3B82F6`), невидимые на тёмной теме. Прописываем токены.
 * `icon-color` подхватывается per-row (Normal/Hover/Selected) — задаём
 * базовый + явный selected-вариант, чтобы выделенный узел подсвечивался
 * акцентом. `border-color` управляет цветом connecting-lines (см.
 * `show_lines(true)` в `right_panel.rs`). */
.syn-explorer-tree {
    background-color: transparent;
    color: var(--text);
    accent-color: var(--primary);
    border-color: var(--border-soft);
    icon-color: var(--text-muted);
    icon-color-selected: var(--primary);
    icon-color-hover: var(--text);
}

/* ─── No-bundle placeholder (центр) ─────────────────────────────── */

.syn-placeholder {
    padding: 40px 28px 40px 28px;
}

.syn-placeholder-icon {
    color: var(--text-subtle);
    font-size: 64px;
}

.syn-placeholder-title {
    color: var(--text);
    font-size: 20px;
    font-weight: 700;
}

.syn-placeholder-hint {
    color: var(--text-muted);
    font-size: 13px;
    max-lines: 2;
}

.syn-placeholder-cta {
    background-color: var(--primary);
    color: var(--on-primary);
    border-radius: var(--syn-radius-input);
    padding: 10px 18px 10px 18px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.syn-placeholder-cta:hover {
    background-color: var(--primary-hover);
}

/* ─── Dialogs ───────────────────────────────────────────────────── */

.syn-dialog-empty {
    background-color: transparent;
}

.syn-dialog-card {
    padding: 22px 24px 20px 24px;
    border-radius: var(--radius-panel);
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--syn-divider);
    min-width: 360px;
    max-width: 480px;
    height: fit-content;
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.45);
}

.syn-dialog-wide {
    min-width: 480px;
    max-width: 640px;
}

.syn-dialog-danger {
    border-color: var(--error);
    border-width: 1px;
}

.syn-dialog-error {
    border-color: var(--error);
    border-width: 1px;
}

.syn-dialog-title {
    color: var(--text);
    font-size: 16px;
    font-weight: 700;
}

.syn-dialog-hint {
    color: var(--text-muted);
    font-size: 13px;
}

.syn-dialog-error-body {
    color: var(--error);
    font-size: 13px;
}

.syn-dialog-label {
    color: var(--text);
    font-size: 12px;
    font-weight: 700;
}

.syn-dialog-path {
    color: var(--text-muted);
    font-size: 12px;
    font-family: monospace;
}

.syn-dialog-input {
    background-color: var(--syn-input-bg);
    border-radius: var(--syn-radius-input);
    border-width: 1px;
    border-color: var(--syn-divider);
    padding: 8px 12px 8px 12px;
}

.syn-dialog-btn-primary {
    background-color: var(--primary);
    color: var(--on-primary);
    border-radius: var(--syn-radius-input);
    padding: 8px 16px 8px 16px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.syn-dialog-btn-primary:hover {
    background-color: var(--primary-hover);
}

.syn-dialog-btn-secondary {
    background-color: var(--syn-card-bg);
    color: var(--text);
    border-radius: var(--syn-radius-input);
    padding: 8px 16px 8px 16px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.syn-dialog-btn-secondary:hover {
    background-color: var(--syn-card-bg-hover);
}

.syn-dialog-btn-danger {
    background-color: var(--error);
    color: var(--on-primary);
    border-radius: var(--syn-radius-input);
    padding: 8px 16px 8px 16px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.syn-dialog-btn-danger:hover {
    background-color: var(--error-hover);
}

/* Заголовок группы внутри диалога (например «Компоненты»). */
.syn-dialog-section-label {
    color: var(--text);
    font-size: 12px;
    font-weight: 700;
    margin-top: 4px;
}

/* Карточка одного компонента в форме создания мульти-tensor пакета. */
.syn-dialog-component-card {
    background-color: var(--syn-card-bg);
    border-radius: var(--syn-radius-input);
    border-width: 1px;
    border-color: var(--syn-divider);
    padding: 10px 12px 10px 12px;
    transition: border-color var(--duration-fast) var(--ease-standard);
}

.syn-dialog-component-card:hover {
    border-color: var(--primary);
}

.syn-dialog-component-index {
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 700;
    font-family: monospace;
    min-width: 22px;
}

/* Icon-only кнопки (удалить компонент). */
.syn-dialog-btn-icon {
    background-color: transparent;
    color: var(--text-muted);
    border-radius: var(--syn-radius-input);
    padding: 6px 8px 6px 8px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.syn-dialog-btn-icon:hover {
    background-color: var(--syn-card-bg-hover);
    color: var(--error);
}

.syn-dialog-checkbox {
    color: var(--text);
    font-size: 13px;
    margin-top: 6px;
}

/* Прогресс-карточка во время сжатия. */
.syn-dialog-progress-card {
    background-color: var(--syn-card-bg);
    border-radius: var(--syn-radius-input);
    border-width: 1px;
    border-color: var(--primary);
    padding: 12px 14px 12px 14px;
    transition: border-color var(--duration-fast) var(--ease-standard);
}

.syn-dialog-progress-stage {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
}

.syn-dialog-progress {
    /* ProgressBar высота / стиль наследует общий progress-bar стиль. */
    height: 8px;
    border-radius: 4px;
}

.syn-dialog-progress-bytes {
    color: var(--text-muted);
    font-size: 12px;
    font-family: monospace;
}

.syn-dialog-progress-percent {
    color: var(--primary);
    font-size: 12px;
    font-weight: 700;
    font-family: monospace;
}
