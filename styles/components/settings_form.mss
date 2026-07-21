/* Формы на странице «Общие»: карточка-таблица с разделителями. */

.settings-row {
    background-color: transparent;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.settings-row:hover {
    background-color: var(--surface-hover);
}

.settings-row-icon-wrap {
    width: 40px;
    height: 40px;
    border-radius: 10px;
    background-color: var(--primary-soft);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.settings-row-icon {
    color: var(--primary);
    icon-size: 20px;
}

.settings-row-title {
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
}

.settings-row-desc {
    color: var(--text-muted);
    font-size: 13px;
}

/* Dropdown в строке настроек — визуально согласован с TextField (тот же
 * `--bg-search` фон + `--border-soft` бордер + 10px радиус). `accent-color`
 * красит chevron-стрелку и обводку открытого/наведённого состояния в
 * `--primary` темы; `--popup-*` синхронизирует выпадающий список с темой
 * (фон `--surface`, hover/selected — `--primary` accents). */
.settings-row-dropdown {
    min-width: 240px;
    background-color: var(--bg-search);
    color: var(--text);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 10px 14px;
    font-size: 14px;
    accent-color: var(--primary);
    /* `--surface` в темах synthos не определена → ранее var(--surface) не
     * резолвился, popup_bg фолбэчился на `background-color` Dropdown'а
     * (`var(--bg-search)` = #F4F4F7), а popup_hover_bg = `--surface-hover`
     * (#F3F3F5) — разница на 1 единицу, hover-цвет был визуально не отличим.
     * Берём существующий «белый/панельный» фон, он гарантированно
     * контрастен с `--surface-hover`. */
    --popup-background: var(--bg-shell);
    --popup-color: var(--text);
    --popup-border: var(--border);
    --popup-accent: var(--primary);
    --popup-hover-background: var(--surface-hover);
    --popup-hover-color: var(--text);
    --popup-selected-background: var(--primary-soft);
    --popup-selected-color: var(--primary);
    transition: border-color var(--duration-fast) var(--ease-standard);
}

.settings-row-dropdown:hover {
    border-color: var(--primary);
}

