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

/* Цвета берутся из глобального правила `Checkbox` в `base/reset.mss` —
 * здесь только размер и отступ. */
.syn-dialog-checkbox {
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

/* ─── Модели, готовые к упаковке ────────────────────────────────────
 * Вторая группа в левой панели: то, что ещё не бандл, но может им стать.
 * Отличается от `.syn-bundle-card` пунктирной рамкой — «черновик», а не
 * готовый пакет.
 * ─────────────────────────────────────────────────────────────────── */

.syn-folder-group-header {
    padding: 10px 14px 4px 14px;
}

.syn-folder-group-title {
    color: var(--text-subtle);
    font-size: 11px;
    font-weight: 600;
}

.syn-source-card {
    padding: 12px 10px 12px 14px;
    border-radius: var(--syn-radius-card);
    background-color: transparent;
    border-width: 1px;
    border-color: var(--syn-divider);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.syn-source-card:hover {
    background-color: var(--syn-card-bg-hover);
    border-color: var(--border-strong);
}

.syn-source-card-icon {
    color: var(--text-subtle);
    font-size: 22px;
}

.syn-source-card-name {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
}

.syn-source-card-meta {
    color: var(--text-muted);
    font-size: 11px;
}

.syn-source-card-action {
    color: var(--primary);
    font-size: 20px;
}

/* ─── Карточка упаковки и мастер ────────────────────────────────────── */

.syn-pack-summary {
    padding: 14px 16px 14px 16px;
    border-radius: var(--syn-radius-card);
    background-color: var(--syn-card-bg);
}

.syn-pack-summary-title {
    color: var(--text);
    font-size: 16px;
    font-weight: 700;
}

.syn-pack-summary-line {
    color: var(--text-muted);
    font-size: 12px;
}

.syn-pack-space {
    color: var(--text-subtle);
    font-size: 11px;
}

.syn-pack-note {
    color: var(--text-subtle);
    font-size: 11px;
}

.syn-pack-warning {
    color: var(--warning);
    font-size: 12px;
}

.syn-guess-badge {
    color: var(--text-subtle);
    font-size: 10px;
}

/* Карточка мастера растёт по содержимому: общей прокрутки здесь нет.
 * ScrollView в syngui всегда занимает всю выданную высоту, поэтому один
 * внешний скролл раздувал бы диалог даже на пустом первом шаге. Вместо
 * него ограничены сами длинные списки — файлы и роли слоёв. */
.syn-wizard-body {
    padding: 4px 0 4px 0;
}

/* ─── Инспектор слоёв ───────────────────────────────────────────────
 * Полоса состава: доля роли задаётся `flex-grow` через классы
 * `.syn-flex-1 … .syn-flex-100` (per-child flex в виджетах не задать, а
 * ширина сегмента зависит от ширины панели). Сто правил — цена за то,
 * чтобы полоса тянулась вместе с окном.
 * ─────────────────────────────────────────────────────────────────── */

:root {
    --syn-role-embedding:    var(--avatar-blue);
    --syn-role-lm-head:      var(--primary);
    --syn-role-attention:    var(--avatar-violet);
    --syn-role-mlp:          var(--avatar-green);
    --syn-role-norm:         var(--border-strong);
    --syn-role-conv:         var(--avatar-slate);
    --syn-role-conditioning: var(--avatar-orange);
    --syn-role-head:         var(--avatar-rose);
    --syn-role-vision:       var(--presence-telegram);
    --syn-role-audio:        var(--presence-whatsapp);
    --syn-role-vae:          var(--warning);
    --syn-role-lora:         var(--presence-instagram);
    --syn-role-other:        var(--text-subtle);
}

.syn-role-bar {
    height: 10px;
    border-radius: var(--syn-radius-pill);
    background-color: var(--syn-card-bg);
}

/* Сегменты стоят вплотную: зазор прибавлялся к сумме процентов и полоса
 * вылезала за карточку. Скругление — только у всей полосы. */
.syn-role-seg {
    height: 10px;
    background-color: var(--syn-role-other);
}

.syn-role-dot {
    width: 8px;
    height: 8px;
    border-radius: var(--syn-radius-pill);
    background-color: var(--syn-role-other);
}

.syn-role-legend {
    color: var(--text-muted);
    font-size: 11px;
}

.syn-role-seg.role-embedding,    .syn-role-dot.role-embedding    { background-color: var(--syn-role-embedding); }
.syn-role-seg.role-lm_head,      .syn-role-dot.role-lm_head      { background-color: var(--syn-role-lm-head); }
.syn-role-seg.role-attention,    .syn-role-dot.role-attention    { background-color: var(--syn-role-attention); }
.syn-role-seg.role-mlp,          .syn-role-dot.role-mlp          { background-color: var(--syn-role-mlp); }
.syn-role-seg.role-norm,         .syn-role-dot.role-norm         { background-color: var(--syn-role-norm); }
.syn-role-seg.role-conv,         .syn-role-dot.role-conv         { background-color: var(--syn-role-conv); }
.syn-role-seg.role-conditioning, .syn-role-dot.role-conditioning { background-color: var(--syn-role-conditioning); }
.syn-role-seg.role-head,         .syn-role-dot.role-head         { background-color: var(--syn-role-head); }
.syn-role-seg.role-vision,       .syn-role-dot.role-vision       { background-color: var(--syn-role-vision); }
.syn-role-seg.role-audio,        .syn-role-dot.role-audio        { background-color: var(--syn-role-audio); }
.syn-role-seg.role-vae,          .syn-role-dot.role-vae          { background-color: var(--syn-role-vae); }
.syn-role-seg.role-lora,         .syn-role-dot.role-lora         { background-color: var(--syn-role-lora); }

.syn-flex-1 { flex-grow: 1; }
.syn-flex-2 { flex-grow: 2; }
.syn-flex-3 { flex-grow: 3; }
.syn-flex-4 { flex-grow: 4; }
.syn-flex-5 { flex-grow: 5; }
.syn-flex-6 { flex-grow: 6; }
.syn-flex-7 { flex-grow: 7; }
.syn-flex-8 { flex-grow: 8; }
.syn-flex-9 { flex-grow: 9; }
.syn-flex-10 { flex-grow: 10; }
.syn-flex-11 { flex-grow: 11; }
.syn-flex-12 { flex-grow: 12; }
.syn-flex-13 { flex-grow: 13; }
.syn-flex-14 { flex-grow: 14; }
.syn-flex-15 { flex-grow: 15; }
.syn-flex-16 { flex-grow: 16; }
.syn-flex-17 { flex-grow: 17; }
.syn-flex-18 { flex-grow: 18; }
.syn-flex-19 { flex-grow: 19; }
.syn-flex-20 { flex-grow: 20; }
.syn-flex-21 { flex-grow: 21; }
.syn-flex-22 { flex-grow: 22; }
.syn-flex-23 { flex-grow: 23; }
.syn-flex-24 { flex-grow: 24; }
.syn-flex-25 { flex-grow: 25; }
.syn-flex-26 { flex-grow: 26; }
.syn-flex-27 { flex-grow: 27; }
.syn-flex-28 { flex-grow: 28; }
.syn-flex-29 { flex-grow: 29; }
.syn-flex-30 { flex-grow: 30; }
.syn-flex-31 { flex-grow: 31; }
.syn-flex-32 { flex-grow: 32; }
.syn-flex-33 { flex-grow: 33; }
.syn-flex-34 { flex-grow: 34; }
.syn-flex-35 { flex-grow: 35; }
.syn-flex-36 { flex-grow: 36; }
.syn-flex-37 { flex-grow: 37; }
.syn-flex-38 { flex-grow: 38; }
.syn-flex-39 { flex-grow: 39; }
.syn-flex-40 { flex-grow: 40; }
.syn-flex-41 { flex-grow: 41; }
.syn-flex-42 { flex-grow: 42; }
.syn-flex-43 { flex-grow: 43; }
.syn-flex-44 { flex-grow: 44; }
.syn-flex-45 { flex-grow: 45; }
.syn-flex-46 { flex-grow: 46; }
.syn-flex-47 { flex-grow: 47; }
.syn-flex-48 { flex-grow: 48; }
.syn-flex-49 { flex-grow: 49; }
.syn-flex-50 { flex-grow: 50; }
.syn-flex-51 { flex-grow: 51; }
.syn-flex-52 { flex-grow: 52; }
.syn-flex-53 { flex-grow: 53; }
.syn-flex-54 { flex-grow: 54; }
.syn-flex-55 { flex-grow: 55; }
.syn-flex-56 { flex-grow: 56; }
.syn-flex-57 { flex-grow: 57; }
.syn-flex-58 { flex-grow: 58; }
.syn-flex-59 { flex-grow: 59; }
.syn-flex-60 { flex-grow: 60; }
.syn-flex-61 { flex-grow: 61; }
.syn-flex-62 { flex-grow: 62; }
.syn-flex-63 { flex-grow: 63; }
.syn-flex-64 { flex-grow: 64; }
.syn-flex-65 { flex-grow: 65; }
.syn-flex-66 { flex-grow: 66; }
.syn-flex-67 { flex-grow: 67; }
.syn-flex-68 { flex-grow: 68; }
.syn-flex-69 { flex-grow: 69; }
.syn-flex-70 { flex-grow: 70; }
.syn-flex-71 { flex-grow: 71; }
.syn-flex-72 { flex-grow: 72; }
.syn-flex-73 { flex-grow: 73; }
.syn-flex-74 { flex-grow: 74; }
.syn-flex-75 { flex-grow: 75; }
.syn-flex-76 { flex-grow: 76; }
.syn-flex-77 { flex-grow: 77; }
.syn-flex-78 { flex-grow: 78; }
.syn-flex-79 { flex-grow: 79; }
.syn-flex-80 { flex-grow: 80; }
.syn-flex-81 { flex-grow: 81; }
.syn-flex-82 { flex-grow: 82; }
.syn-flex-83 { flex-grow: 83; }
.syn-flex-84 { flex-grow: 84; }
.syn-flex-85 { flex-grow: 85; }
.syn-flex-86 { flex-grow: 86; }
.syn-flex-87 { flex-grow: 87; }
.syn-flex-88 { flex-grow: 88; }
.syn-flex-89 { flex-grow: 89; }
.syn-flex-90 { flex-grow: 90; }
.syn-flex-91 { flex-grow: 91; }
.syn-flex-92 { flex-grow: 92; }
.syn-flex-93 { flex-grow: 93; }
.syn-flex-94 { flex-grow: 94; }
.syn-flex-95 { flex-grow: 95; }
.syn-flex-96 { flex-grow: 96; }
.syn-flex-97 { flex-grow: 97; }
.syn-flex-98 { flex-grow: 98; }
.syn-flex-99 { flex-grow: 99; }
.syn-flex-100 { flex-grow: 100; }

/* ─── Выбор точности по ролям ───────────────────────────────────────── */

.syn-quant-row {
    padding: 4px 0 4px 0;
}

.syn-quant-role {
    color: var(--text);
    font-size: 12px;
}

.syn-quant-pct {
    color: var(--text-muted);
    font-size: 11px;
    width: 44px;
}

.syn-quant-size {
    width: 170px;
}

.syn-quant-na {
    color: var(--text-subtle);
    font-size: 11px;
    width: 150px;
}

.syn-quant-dropdown {
    width: 150px;
}

.syn-layers-table {
    font-size: 12px;
}

/* ─── Режим эксперта ────────────────────────────────────────────────── */

.syn-expert-row {
    padding: 2px 0 6px 26px;
}

.syn-expert-block {
    padding: 6px 0 6px 8px;
    border-radius: var(--syn-radius-tile);
    background-color: var(--syn-card-bg);
}

.syn-quant-total {
    padding: 8px 0 2px 0;
    border-top-width: 1px;
    border-color: var(--syn-divider);
}

.syn-quant-size-before {
    color: var(--text-subtle);
    font-size: 11px;
}

.syn-quant-size-after {
    color: var(--diff-added);
    font-size: 12px;
    font-weight: 600;
}

.syn-quant-arrow {
    color: var(--text-subtle);
    font-size: 14px;
}

.syn-quant-hint {
    color: var(--warning);
    font-size: 10px;
}

/* Высоты подобраны так, чтобы карточка с включённым режимом эксперта
 * укладывалась в окно целиком: прокручиваются только сами списки, а итог
 * размера и кнопки остаются на виду. */
.syn-aux-wrap {
    height: 230px;
    overflow: hidden;
}

.syn-quant-wrap {
    height: 380px;
    overflow: hidden;
}

/* Отступ справа — под полосу прокрутки: без него она наезжает на
 * дропдауны точности и обрезает подписи «not quantized by the engine». */
.syn-quant-rows {
    padding: 0 14px 0 0;
}
