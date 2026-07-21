/* ───────────────────────── Node-editor TabsBar (Zed/VSCode-style) ──────
 * Стилистика повторяет .term-tabs-bar (см. terminal_tabs.mss): тонкий
 * 34px бар, accent-полоса под активной вкладкой, hover-эффекты, ×
 * показывается на hover/active. Своя система классов `.ne-tab-*` —
 * чтобы не пересекаться с терминальными табами и иметь свободу для
 * визуальных правок editor'а в будущем.
 * ─────────────────────────────────────────────────────────────────── */

.ne-tabs-bar {
    background-color: var(--bg-panel);
    border-bottom-width: 1px;
    border-color: var(--border);
    height: 34px;
    padding: 0px 4px 0px 0px;
}

/* Spacer прижимает add-кнопку к началу/середине, оставляя право пустым
 * — visually balance под run-controls'ами в overlay-row выше. */
.ne-tabs-spacer {
    background-color: transparent;
    flex-grow: 1;
}

/* ───────────────────────── Один таб ────────────────────────────────── */

.ne-tab {
    /* Фиксированная высота: после удаления ToolButton.size() builder'а
     * intrinsic-высота больше не «тянется» вверх — задаём её здесь
     * явно, чтобы вкладка не схлопывалась. */
    height: 33px;
    /* shrink-to-fit по контенту; реальный лимит ширины — `max-width: 300px`
     * на `.ne-tab-title`. Без fit-content Row родителя растягивал вкладку
     * на всю ширину bar'а и close-кнопка уезжала. */
    width: fit-content;
    background-color: transparent;
    padding: 0px 12px 0px 10px;
    border-right-width: 1px;
    border-color: var(--border);
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);
}
.ne-tab:hover {
    background-color: var(--bg-hover);
}

.ne-tab--active {
    background-color: var(--bg-elevated);
    border-color: var(--border);
    border-bottom-width: 2px;
    border-bottom-color: var(--primary);
}
.ne-tab--active:hover {
    background-color: var(--bg-elevated);
}

/* ───────────────────────── Контент таба ─────────────────────────────── */

.ne-tab-icon {
    font-family: "Material Icons";
    font-size: 16px;
    color: var(--text-muted);
    transition: color var(--duration-fast) var(--ease-standard);
}
.ne-tab--active .ne-tab-icon {
    color: var(--text);
}

.ne-tab-title {
    background-color: transparent;
    /* shrink-to-fit без min-width — иначе fit-content родителя стрижёт по
     * min-content и короткие заголовки обрезаются. */
    max-width: 300px;
}
.ne-tab-title-text {
    color: var(--text-muted);
    font-size: 13px;
    line-clamp: 1;
    transition: color var(--duration-fast) var(--ease-standard);
}
.ne-tab--active .ne-tab-title-text {
    color: var(--text);
    font-weight: 500;
}
.ne-tab:hover .ne-tab-title-text {
    color: var(--text);
}

/* ───────────────────────── Close-кнопка таба ───────────────────────── */

.ne-tab-close {
    color: var(--text-muted);
    background-color: transparent;
    border-radius: var(--radius-pill);
    padding: 2px 2px 2px 2px;
    icon-size: 14px;
    opacity: 0.0;
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}
.ne-tab:hover .ne-tab-close {
    opacity: 0.6;
}
.ne-tab--active .ne-tab-close {
    opacity: 0.6;
}
.ne-tab-close:hover {
    opacity: 1.0;
    color: var(--text);
    background-color: var(--bg-hover);
}

/* ───────────────────────── Add-кнопка (+) ─────────────────────────── */

.ne-tab-add {
    color: var(--text-muted);
    background-color: transparent;
    border-radius: var(--radius-pill);
    padding: 6px 6px 6px 6px;
    icon-size: 16px;
    opacity: 0.7;
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}
.ne-tab-add:hover {
    opacity: 1.0;
    color: var(--text);
    background-color: var(--bg-hover);
}
