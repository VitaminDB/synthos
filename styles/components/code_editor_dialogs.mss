/* ─────────────── Code editor dialogs (New file / Rename / Delete) ─────
 * Модальные карточки в Portal'е. Все три варианта (text-input + delete-
 * confirm) разделяют общую `.code-editor-dialog-card` обёртку.
 *
 * Все цвета — через дизайн-токены из styles/base/variables.mss.
 * Анимации — fade-in карточки (ease-standard, duration-med).
 * ───────────────────────────────────────────────────────────────────── */

.code-editor-dialog-empty {
    /* Заглушка, когда pending_dialog = None но Reactive ещё не убил card.
     * Нулевой размер, прозрачный фон — визуально не виден. */
    background-color: transparent;
    width: 0;
    height: 0;
}

.code-editor-dialog-card {
    background-color: var(--bg-panel);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 24px 24px 20px 24px;
    min-width: 420px;
    max-width: 600px;
    /* Отделяем от backdrop'а Portal'а лёгкой тенью. */
    animation: code-editor-dialog-in var(--duration-med) var(--ease-standard);
}

.code-editor-dialog-card.code-editor-dialog-danger {
    /* Красная окантовка для delete-confirm — пользователь сразу понимает,
     * что действие необратимо. */
    border-color: var(--error);
}

.code-editor-dialog-title {
    color: var(--text);
    font-size: 18px;
    font-weight: 600;
}

.code-editor-dialog-hint {
    color: var(--text-muted);
    font-size: 13px;
}

.code-editor-dialog-path {
    color: var(--text-subtle);
    font-size: 12px;
    /* Длинные пути не переносим: ellipsis на правом крае через line-clamp. */
    line-clamp: 1;
}

.code-editor-dialog-input {
    background-color: var(--bg-panel);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border-strong);
    padding: 8px 12px 8px 12px;
    color: var(--text);
    font-size: 14px;
    transition: border-color var(--duration-fast) var(--ease-standard);
}
.code-editor-dialog-input:focus {
    border-color: var(--primary);
}

/* ─── Кнопки ─────────────────────────────────────────────────────────── */

.code-editor-dialog-btn-secondary {
    background-color: transparent;
    color: var(--text);
    border-width: 1px;
    border-color: var(--border-strong);
    border-radius: var(--radius-panel);
    padding: 8px 16px 8px 16px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);
}
.code-editor-dialog-btn-secondary:hover {
    background-color: var(--surface-hover);
}

.code-editor-dialog-btn-primary {
    background-color: var(--primary);
    color: var(--on-primary);
    border-radius: var(--radius-panel);
    padding: 8px 16px 8px 16px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);
}
.code-editor-dialog-btn-primary:hover {
    background-color: var(--primary-hover);
}

.code-editor-dialog-btn-danger {
    background-color: var(--error);
    color: var(--text-inverse);
    border-radius: var(--radius-panel);
    padding: 8px 16px 8px 16px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);
}
.code-editor-dialog-btn-danger:hover {
    background-color: var(--error-hover);
}

/* ─── Конфликт внешнего изменения ────────────────────────────────────── */

.code-editor-conflict-card {
    border-color: var(--warning);
    max-width: 720px;
}

.code-editor-conflict-icon {
    color: var(--warning);
    font-size: 22px;
}

.code-editor-diff-scroll {
    background-color: var(--bg-search);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 8px 10px 8px 10px;
    max-height: 320px;
}

.code-editor-diff-line {
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 12px;
    color: var(--text-muted);
    padding: 1px 4px 1px 4px;
}

.code-editor-diff-line.added {
    color: var(--diff-added);
    background-color: var(--diff-added-bg);
}

.code-editor-diff-line.removed {
    color: var(--diff-removed);
    background-color: var(--diff-removed-bg);
}

/* ─── История версий (Local History) ─────────────────────────────────── */

.code-editor-history-card {
    min-width: 460px;
}

.code-editor-history-icon {
    color: var(--primary);
    font-size: 22px;
}

.code-editor-history-scroll {
    max-height: 360px;
}

.code-editor-history-row {
    padding: 8px 10px 8px 10px;
    border-radius: var(--radius-panel);
    background-color: var(--bg-search);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.code-editor-history-row:hover {
    background-color: var(--surface-hover);
}

.code-editor-history-row-icon {
    color: var(--text-subtle);
    font-size: 16px;
}

.code-editor-history-row-age {
    color: var(--text);
    font-size: 13px;
}

.code-editor-history-empty {
    color: var(--text-subtle);
    font-size: 13px;
    padding: 16px 0 16px 0;
}

.code-editor-history-btn {
    color: var(--text-muted);
    transition: color var(--duration-fast) var(--ease-standard);
}

.code-editor-history-btn:hover {
    color: var(--primary);
}

/* ─── Анимация появления ─────────────────────────────────────────────── */

@keyframes code-editor-dialog-in {
    from {
        opacity: 0;
        transform: translateY(8px);
    }
    to {
        opacity: 1;
        transform: translateY(0);
    }
}

/* ─── Подтверждение выхода (components::quit_dialog) ──────────────────
 * Та же карточка, что у остальных подтверждений; отличается списком
 * причин — по строке на каждое незавершённое дело.
 * ───────────────────────────────────────────────────────────────────── */

.quit-dialog {
    max-width: 520px;
}

.quit-dialog-bullet {
    color: var(--warning);
    font-size: 16px;
}

.quit-dialog-reason {
    color: var(--text);
    font-size: 14px;
}
