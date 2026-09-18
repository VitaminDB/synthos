/* Правая панель: детали модели — README + Files + per-file прогресс. */

.hf-detail-panel {
    background-color: var(--bg-shell);
    height: 100%;
}

.hf-detail-header {
    padding: var(--spacing-md) var(--spacing-lg);
    border-bottom-width: 1px;
    border-bottom-color: var(--border-soft);
}

.hf-detail-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
    line-clamp: 1;
}

.hf-detail-subtitle {
    font-size: 12px;
    color: var(--text-muted);
}

/* Letter-avatar 32×32 рядом с repo_id заголовком. */
.hf-detail-avatar-letter {
    width: 32px;
    height: 32px;
    border-radius: 16px;
    overflow: hidden;
}

.hf-detail-avatar-letter-text {
    font-size: 14px;
    font-weight: 600;
    color: #FFFFFF;
}

.hf-detail-avatar-fallback {
    width: 32px;
    height: 32px;
    border-radius: 16px;
    background-color: var(--primary-soft);
    padding: 4px;
}

.hf-detail-avatar-icon {
    icon-size: 22px;
    color: var(--primary);
}

.hf-detail-tabs {
    padding: 0 var(--spacing-md);
    border-bottom-width: 1px;
    border-bottom-color: var(--border-soft);
}

/* Заголовок правой панели файлов — на месте бывшего TabBar. */
.hf-files-panel-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
    padding: 12px 0 10px 0;
}

.hf-detail-content {
    padding: var(--spacing-md);
    height: 100%;
}

.hf-detail-placeholder {
    padding: var(--spacing-xl);
    max-width: 360px;
}

.hf-detail-placeholder-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
}

.hf-detail-placeholder-hint {
    font-size: 12px;
    color: var(--text-muted);
    text-align: center;
}

/* README */

.hf-readme-scroll {
    height: 100%;
}

.hf-readme {
    padding: var(--spacing-md);
    background-color: var(--bg-shell);
    border-radius: var(--radius-panel);
}

/* MarkdownView — все цвета md-блоков через тему. syngui не каскадит
 * custom-properties вниз, поэтому переменные задаются на самом виджете
 * (см. detail_panel.rs::readme_tab — `.class("hf-md")` на MarkdownView). */
.hf-md {
    color: var(--text);
    font-size: 14px;
    line-height: 1.5;

    /* Headings */
    --md-heading-color: var(--text);
    --md-heading-spacing: 8px;

    /* Inline links и code */
    --md-link-color: var(--primary);
    --md-code-bg: var(--surface-hover);
    --md-code-color: var(--primary);
    --md-code-radius: 6px;

    /* Code-блоки (fenced ```). Лёгкая «карточка» поверх bg-shell,
     * чтобы блок был отличим от тела параграфа. */
    --md-code-block-bg: var(--bg-window);
    --md-code-block-color: var(--text);
    --md-code-block-radius: 8px;
    --md-code-block-padding: 12px;

    /* Copy-кнопка над code-блоком */
    --md-copy-bg: var(--surface-hover);
    --md-copy-bg-hover: var(--surface-selected);
    --md-copy-color: var(--text-muted);
    --md-copy-flash-bg: var(--presence-online);

    /* Цитаты */
    --md-quote-bg: var(--surface-hover);
    --md-quote-text-color: var(--text-muted);
    --md-quote-border-color: var(--primary);
    --md-quote-border-width: 3px;
    --md-quote-radius: 6px;

    /* Списки + чекбоксы */
    --md-list-indent: 20px;
    --md-bullet-color: var(--text-muted);
    --md-checkbox-color: var(--primary);
    --md-checkbox-check-color: var(--on-primary);

    /* Таблицы */
    --md-table-border-color: var(--border-soft);
    --md-table-header-bg: var(--surface-hover);
    --md-table-header-color: var(--text);
    --md-table-stripe-bg: var(--surface-hover);

    /* Горизонтальные линии и прочее */
    --md-hr-color: var(--border-soft);
    --md-strikethrough-color: var(--text-muted);

    /* Картинки-плейсхолдеры */
    --md-image-placeholder-bg: var(--surface-hover);
    --md-image-placeholder-color: var(--text-subtle);

    /* Сноски */
    --md-footnote-color: var(--text-muted);

    /* Выделение текста — единый цвет под акцент темы */
    selection-color: var(--primary-soft);
}

.hf-readme-loading,
.hf-readme-empty {
    color: var(--text-subtle);
    font-size: 13px;
}

/* Files */

.hf-files-scroll {
    height: 100%;
}

.hf-files-loading,
.hf-files-empty {
    color: var(--text-subtle);
    font-size: 13px;
}

.hf-file-row {
    padding: var(--spacing-md);
    border-radius: 12px;
    border-width: 1px;
    border-color: var(--border-soft);
    background-color: var(--bg-shell);
    transition: border-color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}

.hf-file-row:hover {
    border-color: var(--border-strong);
    background-color: var(--surface-hover);
}

/* Чекбокс выбора файла слева в строке — не растягивается. */
.hf-file-checkbox {
    margin-right: 2px;
}

.hf-file-name {
    font-size: 13px;
    color: var(--text);
    font-weight: 500;
    line-clamp: 1;
}

.hf-file-size {
    font-size: 12px;
    color: var(--text-muted);
}

/* Скорость загрузки активного файла. Лёгкий primary-цвет, моноширинный
 * шрифт чтобы цифры не «прыгали» при обновлении каждые 500мс. */
.hf-file-speed {
    font-size: 12px;
    color: var(--primary);
    font-weight: 500;
    font-family: monospace;
}

.hf-file-speed-empty {
    height: 0;
    width: 0;
}

.hf-file-spacer {
    height: 1px;
}

.hf-file-dl-btn {
    padding: 6px 14px;
    background-color: var(--primary-soft);
    color: var(--primary);
    border-radius: var(--radius-pill);
    font-weight: 500;
    font-size: 12px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.hf-file-dl-btn:hover {
    background-color: var(--primary);
    color: var(--on-primary);
}

/* Вторичные действия (Пауза/Стоп/Отмена) — нейтральная плашка, компактнее
 * основной «Скачать». */
.hf-file-action-btn {
    padding: 6px 12px;
    background-color: var(--surface-hover);
    color: var(--text);
    border-radius: var(--radius-pill);
    font-weight: 500;
    font-size: 12px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.hf-file-action-btn:hover {
    background-color: var(--surface-selected);
}

/* Отмена — деструктивный акцент на hover. */
.hf-file-action-btn.cancel:hover {
    background-color: rgba(239, 68, 68, 0.18);
    color: var(--error);
}

/* Status icons */

.hf-status-empty {
    width: 22px;
    height: 22px;
}

.hf-status-done {
    width: 22px;
    height: 22px;
    background-color: var(--presence-online);
    border-radius: var(--radius-pill);
    padding: 3px;
}

.hf-status-error {
    width: 22px;
    height: 22px;
    background-color: var(--error);
    border-radius: var(--radius-pill);
    padding: 3px;
}

.hf-status-icon {
    icon-size: 14px;
    color: var(--text-inverse);
}

/* Progress bar — высота 10px, контрастный rail (slate с прозрачностью),
 * яркий primary-fill. Видно одинаково и на светлой, и на тёмной теме. */

.hf-progress-empty {
    height: 0;
}

.hf-progress {
    height: 10px;
    background-color: rgba(148, 163, 184, 0.22);
    border-radius: 5px;
    overflow: hidden;
}

.hf-progress-bar {
    height: 10px;
    background-color: var(--primary);
    border-radius: 5px;
    transition: width var(--duration-fast) var(--ease-standard);
}

.hf-progress-indeterminate .hf-progress-bar {
    /* Без width — заполняет родителя, opacity-пульсация даёт «дыхание». */
    width: 100%;
    opacity: 0.85;
    animation: hf-progress-pulse 1.6s ease-in-out infinite;
}

/* Pending — задача в очереди: едва видимый rail. */
.hf-progress-pending {
    background-color: rgba(148, 163, 184, 0.14);
    opacity: 0.85;
}

/* Сегментированный прогресс — N равных rail'ов, каждый flex-grow:1.
 * Visual: «вагон»-индикатор: видно, что файл качается параллельно.
 * gap 3px между сегментами читается как отдельные блоки. */
.hf-progress-segments {
    height: 10px;
}

.hf-progress-segment {
    height: 10px;
    background-color: rgba(148, 163, 184, 0.22);
    border-radius: 5px;
    overflow: hidden;
}

.hf-progress-segment-fill {
    height: 10px;
    background-color: var(--primary);
    border-radius: 5px;
    transition: width var(--duration-fast) var(--ease-standard);
}

/* Active-режим: лёгкая пульсация всех заполнений — показывает «качается». */
.hf-progress-segments-active .hf-progress-segment-fill {
    animation: hf-progress-pulse 1.6s ease-in-out infinite;
}

@keyframes hf-progress-pulse {
    0%, 100% { opacity: 0.85; }
    50%      { opacity: 1.0; }
}

/* Retry-метка («попытка N/3») в file-row справа от размера. */
.hf-status-retry {
    font-size: 11px;
    color: var(--warning);
    font-weight: 500;
}

/* SHA-256 верификация: бейдж + кнопка ручного запуска. */

.hf-verify-badge-empty {
    width: 0;
    height: 0;
}

.hf-verify-badge {
    padding: 2px 8px;
    border-radius: var(--radius-pill);
    border-width: 1px;
}

.hf-verify-badge.ok {
    background-color: rgba(16, 185, 129, 0.16);
    border-color: rgba(16, 185, 129, 0.55);
}

.hf-verify-badge.info {
    background-color: rgba(148, 163, 184, 0.16);
    border-color: rgba(148, 163, 184, 0.45);
}

.hf-verify-badge.bad {
    background-color: rgba(239, 68, 68, 0.18);
    border-color: rgba(239, 68, 68, 0.6);
}

.hf-verify-badge.computing {
    background-color: rgba(148, 163, 184, 0.18);
    border-color: rgba(148, 163, 184, 0.4);
    animation: hf-progress-pulse 1.6s ease-in-out infinite;
}

.hf-verify-text {
    font-size: 11px;
    font-weight: 500;
    color: var(--text);
    font-family: monospace;
}

.hf-file-verify-btn {
    padding: 6px 12px;
    background-color: var(--surface-hover);
    color: var(--text);
    border-radius: var(--radius-pill);
    font-weight: 500;
    font-size: 12px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.hf-file-verify-btn:hover {
    background-color: var(--surface-selected);
}

.hf-file-verify-empty {
    width: 0;
    height: 0;
}

/* Pending-статус (часики) — серый кружок с иконкой ожидания. */
.hf-status-pending {
    width: 22px;
    height: 22px;
    background-color: var(--text-subtle);
    border-radius: var(--radius-pill);
    padding: 3px;
}

/* Paused/Stopped — приглушённый янтарный/серый кружок: загрузка прервана
 * пользователем, но `.part` сохранён и резюмится. */
.hf-status-paused {
    width: 22px;
    height: 22px;
    background-color: var(--warning);
    border-radius: var(--radius-pill);
    padding: 3px;
}

.hf-status-stopped {
    width: 22px;
    height: 22px;
    background-color: var(--text-muted);
    border-radius: var(--radius-pill);
    padding: 3px;
}

/* Files toolbar над списком */

.hf-files-toolbar {
    padding: 10px var(--spacing-md);
    background-color: var(--bg-shell);
    border-bottom-width: 1px;
    border-bottom-color: var(--border-soft);
    margin-bottom: var(--spacing-sm);
}

.hf-toolbar-dl-all-btn {
    padding: 7px 14px;
    background-color: var(--primary);
    color: var(--on-primary);
    border-radius: var(--radius-pill);
    font-size: 12px;
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.hf-toolbar-dl-all-btn:hover {
    background-color: var(--primary-hover);
}

/* «Скачать выбранные» — основное действие выбора; чуть приглушённее dl-all,
 * чтобы две кнопки не конкурировали по яркости. */
.hf-toolbar-dl-sel-btn {
    padding: 7px 14px;
    background-color: var(--primary-soft);
    color: var(--primary);
    border-radius: var(--radius-pill);
    font-size: 12px;
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.hf-toolbar-dl-sel-btn:hover {
    background-color: var(--primary);
    color: var(--on-primary);
}

/* Чекбокс «Выбрать всё» в тулбаре. */
.hf-toolbar-select-all {
    font-size: 12px;
    color: var(--text-muted);
}

.hf-toolbar-stat-text {
    font-size: 12px;
    color: var(--text-muted);
}

/* Иконочная кнопка 28×28: переключатель вида, действия на плитке и в очереди
 * нижней панели. ToolButton без своих правил рисуется светлым вне темы. */
.hf-icon-btn {
    width: 28px;
    height: 28px;
    icon-size: 16px;
    border-radius: 8px;
    border-width: 0;
    background-color: transparent;
    color: var(--text-muted);
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.hf-icon-btn:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.hf-icon-btn:selected {
    background-color: var(--primary-soft);
    color: var(--primary);
}

.hf-icon-btn.primary {
    background-color: var(--primary-soft);
    color: var(--primary);
}

.hf-icon-btn.primary:hover {
    background-color: var(--primary);
    color: var(--on-primary);
}

.hf-icon-btn.danger:hover {
    background-color: rgba(239, 68, 68, 0.18);
    color: var(--error);
}

/* Переключатель «список / значки» — две кнопки в общей рамке. */
.hf-view-mode {
    padding: 2px;
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    background-color: var(--bg-shell);
}

/* Плитка файла в режиме «значки». Ширина фиксирована: Flex переносит плитки
 * по ширине панели. */
.hf-file-tile {
    width: 148px;
    padding: 8px;
    border-radius: 12px;
    border-width: 1px;
    border-color: var(--border-soft);
    background-color: var(--bg-shell);
    transition: border-color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}

.hf-file-tile:hover {
    border-color: var(--border-strong);
    background-color: var(--surface-hover);
}

.hf-file-tile.active { border-color: var(--primary); }
.hf-file-tile.error  { border-color: var(--error); }

.hf-file-tile-icon {
    icon-size: 40px;
    color: var(--text-subtle);
}

.hf-file-tile.done .hf-file-tile-icon   { color: var(--presence-online); }
.hf-file-tile.active .hf-file-tile-icon { color: var(--primary); }

.hf-file-tile-name {
    font-size: 12px;
    font-weight: 500;
    color: var(--text);
    text-align: center;
    line-clamp: 2;
}

.hf-file-tile-dir {
    font-size: 10px;
    color: var(--text-subtle);
    text-align: center;
    line-clamp: 1;
}

.hf-file-tile-size {
    font-size: 11px;
    color: var(--text-muted);
    text-align: center;
}
