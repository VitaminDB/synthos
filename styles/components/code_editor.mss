/* ───────────────────────── Code editor page ─────────────────────────
 * Трёхпанельный layout: file tree (300px) | center | open files (260px).
 * В центральной колонке — Vertical SplitView: editor pane / terminal stub.
 *
 * Все цвета — через дизайн-токены из styles/base/variables.mss. Никаких
 * хардкод-значений (правило TASK.md). Скругления через --radius-*,
 * переходы — --duration-fast / --ease-standard.
 *
 * Цвета file-type иконок (`.file-icon-*` классы) определены ниже как
 * локальные `--file-color-*` переменные. Палитра вдохновлена Material
 * Icon Theme — насыщенные тоны, читаются и в светлой, и в тёмной теме
 * (без хардкодов в Rust-коде; см. правило TASK.md).
 * ──────────────────────────────────────────────────────────────────── */

:root {
    /* ── File-type палитра. Используется для классов `.file-icon-*`,
     * присвоенных Icon-виджетам в open_files и заголовке editor_pane.
     * TreeView с релиза 2026-04-30 поддерживает per-node decoration
     * (label/icon color, badge dot, strikethrough) — git-status
     * подсветка в дереве реализована через
     * `pages::code_editor::git_status::apply_to_nodes`, цвета берёт
     * из `--git-color-*` ниже (зеркалит этот блок в Rust-код). */
    --file-color-folder:   #79B4FF;
    --file-color-code:     #FFB454;
    --file-color-json:     #CBD33C;
    --file-color-toml:     #B07FFF;
    --file-color-markdown: #56A8F5;
    --file-color-text:     #B0B7BF;
    --file-color-image:    #E07AB1;
    --file-color-video:    #F47ABF;
    --file-color-audio:    #7AA2F7;
    --file-color-font:     #C792EA;
    --file-color-pdf:      #EE5E48;
    --file-color-archive:  #C39B70;
    --file-color-lock:     #E0B341;
    --file-color-shell:    #6BD36B;
    --file-color-dotfile:  #9CA3AF;
    --file-color-other:    #8C8E92;

    /* ── Git-status палитра. Theme-agnostic, читается на любом фоне.
     * Зеркалит `git_status::GitPalette::default()` в Rust-коде:
     * правка ОБЕИХ сторон одновременно, иначе цвет в дереве разъедется
     * с цветом статуса в тулбаре или будущей source-control панели. */
    --git-color-modified:  #FFB454;   /* yellow, ровно --file-color-code */
    --git-color-new:       #6BD36B;   /* green,  ровно --file-color-shell */
    --git-color-deleted:   #EE5E48;   /* red,    ровно --file-color-pdf */
    --git-color-conflict:  #B07FFF;   /* purple, ровно --file-color-toml */
}

.code-editor-page {
    background-color: var(--bg-shell);
}

.code-editor-center {
    background-color: var(--bg-shell);
}

/* ───────────────────────── Левая панель (File tree) ───────────────── */

.code-editor-tree-panel {
    background-color: var(--bg-panel);
    /* Ширину панели задаёт горизонтальный SplitView через ratio-сигнал
     * `code_editor_left_split_ratio` (см. `pages::code_editor::view`).
     * Border убран — его роль играет drag-дивайдер `.code-editor-h-split`. */
    border-color: var(--border-soft);
}

/*
 * Все три шапки (tree / edit / files) обязаны быть одной высоты — иначе
 * между панелями получится визуальный «ступенчатый» bottom-border, что
 * пользователь и заметил. Фиксированная min-height: 48px (Material standard)
 * + одинаковый горизонтальный padding гарантируют визуальное выравнивание
 * вне зависимости от content'а каждой шапки.
 */
.code-editor-tree-header {
    height: 48px;
    padding: 8px 14px 8px 16px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.code-editor-tree-folder-name {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
}

/*
 * Невидимый spacer внутри Row header'а — растягивается до правого края
 * через flex-grow и прижимает ToolButton к нему.
 */
.code-editor-header-spacer {
    background-color: transparent;
    flex-grow: 1;
}

/*
 * Иконочная open-folder ToolButton — стиль идентичен остальным action-icon
 * кнопкам synthos (`.term-tab-add`, `.term-gear-btn`): transparent idle,
 * hover подсвечивает фон через `--bg-hover`. Без primary-фона — чтобы
 * визуально не доминировала над контентом панели.
 */
.code-editor-open-folder-btn {
    color: var(--text-muted);
    background-color: transparent;
    border-radius: var(--radius-pill);
    padding: 6px 6px 6px 6px;
    icon-size: 18px;
    opacity: 0.7;
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}
.code-editor-open-folder-btn:hover {
    opacity: 1.0;
    color: var(--text);
    background-color: var(--bg-hover);
}

.code-editor-tree-body {
    padding: 8px 8px 8px 8px;
}

.code-editor-empty-icon {
    icon-size: 32px;
    color: var(--text-subtle);
}
.code-editor-empty-title {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
}
.code-editor-empty-hint {
    color: var(--text-muted);
    font-size: 12px;
}

/* ───────────────────────── Центр / редактор ───────────────────────── */

.code-editor-edit-pane {
    background-color: var(--bg-panel);
}

.code-editor-edit-header {
    height: 48px;
    padding: 8px 14px 8px 14px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.code-editor-edit-icon {
    icon-size: 18px;
    color: var(--text-muted);
}

.code-editor-edit-filename {
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
}

/* Маленький круглый индикатор «есть несохранённые правки». В неактивном
 * состоянии прозрачный — занимает место, но не виден. */
.code-editor-dirty-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background-color: transparent;
    transition: background-color var(--duration-fast) var(--ease-standard);
}
.code-editor-dirty-dot.active {
    background-color: var(--primary);
}

/* Save-кнопка по умолчанию приглушена; при dirty подсвечивается. */
.code-editor-save-btn {
    color: var(--text-subtle);
    border-radius: var(--radius-pill);
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}
.code-editor-save-btn:hover {
    background-color: var(--surface-hover);
}
.code-editor-save-btn.enabled {
    color: var(--primary);
}
.code-editor-save-btn.enabled:hover {
    background-color: var(--primary-soft);
}

/* Word-wrap toggle — идентичная логика visual feedback что у save-btn:
 * приглушённый по умолчанию, primary-color подсветка когда активен.
 * Иконка wrap_text меняет состояние сразу (без dirty-условия). */
.code-editor-wrap-btn {
    color: var(--text-subtle);
    border-radius: var(--radius-pill);
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}
.code-editor-wrap-btn:hover {
    background-color: var(--surface-hover);
}
.code-editor-wrap-btn.enabled {
    color: var(--primary);
}
.code-editor-wrap-btn.enabled:hover {
    background-color: var(--primary-soft);
}

.code-editor-edit-body {
    padding: 0;
}

/* CodeEditor на странице code — пустышка для будущих локальных оверрайдов
 * (если понадобится отличающаяся геометрия/padding). Все цвета и token-*
 * палитра уже привязаны к теме глобальным `CodeEditor {}` в base/reset.mss.
 */
.code-editor-mle {
    /* intentionally empty — наследует от глобального CodeEditor */
}

.code-editor-edit-empty-icon {
    icon-size: 36px;
    color: var(--text-subtle);
}
.code-editor-edit-empty-title {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
}
.code-editor-edit-empty-hint {
    color: var(--text-muted);
    font-size: 12px;
}

/* ───────────────────────── Терминал (заглушка) ────────────────────── */

.code-editor-terminal-pane {
    background-color: var(--bg-panel);
    /*
     * border-top — единственная видимая линия между editor-pane и
     * terminal-pane (splitter скрыт через `.code-editor-split`,
     * `divider-thickness:0` + `accent-color:transparent`). Drag-зона
     * сплиттера остаётся: курсор-resize при наведении на 6px-полосу.
     */
    border-top-width: 1px;
    border-color: var(--border-soft);
    animation: code-editor-fade-in var(--duration-med) var(--ease-standard);
}

.code-editor-terminal-icon {
    icon-size: 32px;
    color: var(--text-subtle);
}
.code-editor-terminal-title {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
}
.code-editor-terminal-hint {
    color: var(--text-muted);
    font-size: 12px;
}

/* ───────────────────────── Правая панель (open files) ─────────────── */

.code-editor-files-panel {
    background-color: var(--bg-panel);
    /* Ширину панели задаёт горизонтальный SplitView через ratio-сигнал
     * `code_editor_right_split_ratio` (см. `pages::code_editor::view`).
     * Border убран — его роль играет drag-дивайдер `.code-editor-h-split`. */
    border-color: var(--border-soft);
}

.code-editor-files-header {
    height: 48px;
    padding: 8px 14px 8px 16px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}
.code-editor-files-title {
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 600;
}

.code-editor-files-body {
    padding: 6px 6px 6px 6px;
}

.code-editor-files-list {
    background-color: transparent;
    border-color: transparent;
}

.code-editor-files-item {
    background-color: transparent;
    border-radius: var(--radius-panel);
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);
}
.code-editor-files-item:hover {
    background-color: var(--surface-hover);
}
.code-editor-files-item.selected {
    background-color: var(--surface-selected);
}

.code-editor-files-item-icon {
    icon-size: 16px;
    /* fallback цвет если class_for_path не дал ни одного из `.file-icon-*` —
     * для этого нужно, чтобы общий `.code-editor-files-item-icon` остался
     * читаемым на selected/hover фонах. */
    icon-color: var(--text-muted);
}

/* ───────────────────────── File-type icons ───────────────────────── */
/* Цвета по группам типов файлов. Используются и в open_files (вкладки),
 * и в editor_pane header. TreeView в file_tree берёт «нейтральный» цвет
 * из `.code-editor-tree { icon-color }`; per-file подсветка по типу
 * (выделять `.rs` оранжевым и т. п.) — задача отдельного sprint'а
 * (требует расширения decoration → класс или по-расширения класса
 * TreeNode). Сейчас на TreeView отрисовывается только git-status. */
.file-icon-folder   { icon-color: var(--file-color-folder); }
.file-icon-code     { icon-color: var(--file-color-code); }
.file-icon-json     { icon-color: var(--file-color-json); }
.file-icon-toml     { icon-color: var(--file-color-toml); }
.file-icon-markdown { icon-color: var(--file-color-markdown); }
.file-icon-text     { icon-color: var(--file-color-text); }
.file-icon-image    { icon-color: var(--file-color-image); }
.file-icon-video    { icon-color: var(--file-color-video); }
.file-icon-audio    { icon-color: var(--file-color-audio); }
.file-icon-font     { icon-color: var(--file-color-font); }
.file-icon-pdf      { icon-color: var(--file-color-pdf); }
.file-icon-archive  { icon-color: var(--file-color-archive); }
.file-icon-lock     { icon-color: var(--file-color-lock); }
.file-icon-shell    { icon-color: var(--file-color-shell); }
.file-icon-dotfile  { icon-color: var(--file-color-dotfile); }
.file-icon-other    { icon-color: var(--file-color-other); }
.code-editor-files-item-name {
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
}

.code-editor-files-item-dirty {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background-color: transparent;
    transition: background-color var(--duration-fast) var(--ease-standard);
}
.code-editor-files-item-dirty.active {
    background-color: var(--primary);
}

.code-editor-files-item-close {
    color: var(--text-subtle);
    border-radius: var(--radius-pill);
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}
.code-editor-files-item-close:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.code-editor-files-empty-icon {
    icon-size: 28px;
    color: var(--text-subtle);
}
.code-editor-files-empty-title {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
}
.code-editor-files-empty-hint {
    color: var(--text-muted);
    font-size: 12px;
}

/* ───────────────────────── TreeView (file tree) ───────────────────── */

/* Сам TreeView рисует только подсветку строк (selected/hover) и текст;
 * фон/рамка дерева — задача parent-панели `.code-editor-tree-body`.
 * `accent-color` управляет цветом выделенной строки и подсветкой при
 * наведении на стрелочку — он берётся из активной темы (`--primary`),
 * поэтому дерево автоматически перекрашивается при смене темы. */
.code-editor-tree {
    background-color: transparent;
    color: var(--text);
    border-color: transparent;
    accent-color: var(--primary);
    /* Иконки в дереве пока без per-node цвета (TreeNode не имеет .class).
     * Общий `--text-muted` обеспечивает читаемый mid-contrast — иконка
     * не теряется ни на normal, ни на hover/selected фоне. Различение
     * типов остаётся через codepoint Material Icons (code/data_object/
     * lock/folder/etc.). */
    icon-color: var(--text-muted);
}

/* ───────────────────────── SplitView (editor ↔ terminal) ──────────── */

/* `divider_width(6)` в коде задаёт hit-area (комфортно попасть мышью),
 * визуально дивайдер скрыт: терминал и редактор на одинаковом фоне
 * `--bg-panel`, разделение задано border-bottom'ом таб-бара терминала.
 * Drag-зона остаётся (cursor становится Resize при наведении).
 */
.code-editor-split {
    accent-color: transparent;
    color: transparent;
    divider-thickness: 0;
}

/*
 * Горизонтальные дивайдеры левой/правой колонок (file_tree | center | open_files).
 * Drag-зона — 6px (см. `divider_width(6.0)` в Rust), визуальная полоска — 1px:
 * выглядит как обычный border между панелями, но кликабельна по всей ширине.
 * Hover/drag подсветка достаётся встроенным `accent-color` логикой SplitView.
 */
.code-editor-h-split {
    border-color: var(--border-soft);
    accent-color: var(--primary);
    divider-thickness: 1px;
}

/* ───────────────────────── No-session placeholder ─────────────────── */

/*
 * Показывается на странице `code`, когда у пользователя нет ни одной
 * открытой сессии редактора кода (свежая установка или закрыли все).
 * Подсказывает кликнуть на "+" в боковой панели.
 */
.code-editor-no-session-icon {
    color: var(--primary);
    icon-size: 64px;
    transition: color var(--duration-med) var(--ease-standard);
}

.code-editor-no-session-title {
    font-size: 20px;
    font-weight: 600;
    color: var(--text);
    transition: color var(--duration-med) var(--ease-standard);
}

.code-editor-no-session-hint {
    font-size: 13px;
    color: var(--text-muted);
    max-width: 480px;
    text-align: center;
    transition: color var(--duration-med) var(--ease-standard);
}

/* ───────────────────────── Анимации ───────────────────────────────── */

@keyframes code-editor-fade-in {
    from {
        opacity: 0;
    }
    to {
        opacity: 1;
    }
}
