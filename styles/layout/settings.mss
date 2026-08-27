/* Компоновка страницы настроек.
 * settings-shell наследует фон основной области чата (--bg-chat).
 * settings-right зеркалит грамматику .customer-panel с главной:
 *   width 300px, height 100%, белый фон, border-left.
 */

.settings-shell {
    background-color: var(--bg-chat);
    transition: background-color var(--duration-med) var(--ease-standard);
}

.settings-page {
    background-color: transparent;
}

.settings-page-title {
    font-size: 24px;
    font-weight: 700;
    color: var(--text);
}

.settings-page-subtitle {
    font-size: 14px;
    color: var(--text-muted);
}

.settings-section-title {
    font-size: 13px;
    font-weight: 700;
    color: var(--text-subtle);
    letter-spacing: 0.6px;
}

.settings-card {
    background-color: var(--bg-panel);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border);
    /* Clip по скруглённым углам: без него border-bottom разделителей
     * строк (settings-row / models-active-row) торчит за контур карты. */
    overflow: hidden;
    transition: border-color var(--duration-med) var(--ease-standard),
                background-color var(--duration-med) var(--ease-standard);
}

/* Правая панель настроек — зеркало .settings-sidebar: те же width/height,
 * внешний padding и border со стороны, обращённой к центральной области.
 * Внутренние панели (.skills-panel / .models-panel) — прозрачные, фон
 * и отступы контролирует этот wrapper. */
.settings-right {
    /* Ширину задаёт SplitView (`settings_right_split_ratio`). */
    height: 100%;
    background-color: var(--bg-panel);
    border-left-width: 1px;
    border-color: var(--border-soft);
    padding: 24px 8px 16px 8px;
    transition: background-color var(--duration-med) var(--ease-standard),
                border-color var(--duration-med) var(--ease-standard);
}

.settings-right-hint-bubble {
    width: 72px;
    height: 72px;
    border-radius: 999px;
    background-color: var(--primary-soft);
}

.settings-right-hint-icon {
    color: var(--primary);
    icon-size: 32px;
}

.settings-right-hint-title {
    font-size: 16px;
    font-weight: 700;
    color: var(--text);
}

.settings-right-hint-text {
    font-size: 13px;
    color: var(--text-muted);
    text-align: center;
}

/* Центр настроек внутри каркаса: занимает весь слот между панелями. */
.settings-content {
    height: 100%;
}

/* ─── Раздел «Архив» ─────────────────────────────────────────────── */
.settings-archive-row {
    padding: 12px 16px 12px 16px;
}

.settings-archive-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
}

.settings-archive-preview {
    font-size: 12px;
    color: var(--text-muted);
}

.settings-archive-meta {
    font-size: 11px;
    color: var(--text-subtle);
}

.settings-archive-restore {
    border-radius: 10px;
    padding: 6px 12px;
    background-color: var(--primary-soft);
    color: var(--primary);
    font-size: 12px;
    font-weight: 600;
}

.settings-archive-restore:hover {
    background-color: var(--primary);
    color: var(--on-primary);
}

.settings-archive-delete {
    color: var(--text-muted);
    border-radius: 10px;
}

.settings-archive-delete:hover {
    color: var(--error);
    background-color: var(--surface-hover);
}

.settings-archive-clear {
    border-radius: 10px;
    padding: 6px 12px;
    background-color: transparent;
    border-width: 1px;
    border-color: var(--border);
    color: var(--text-muted);
    font-size: 12px;
}

.settings-archive-clear:hover {
    color: var(--error);
    border-color: var(--error);
}

.settings-archive-empty-icon {
    icon-size: 40px;
    color: var(--text-subtle);
}

.settings-archive-empty-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
}

.settings-archive-empty-hint {
    font-size: 13px;
    color: var(--text-muted);
}
