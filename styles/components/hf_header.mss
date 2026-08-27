/* Поиск по Hub (поле — в общей шапке, чипы — над списком моделей). */

/* Search bar */

/* TextField сам рендерится как DecoratedBox с border/padding/radius из MSS —
 * не оборачиваем его дополнительно, чтобы не получить двойную рамку. Класс
 * `.hf-search-bar` лишь тонко настраивает фон/радиус/ширину. */
.hf-search-bar {
    width: 100%;
    background-color: var(--bg-search);
    border-radius: var(--radius-pill);
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 4px 14px;
    transition: border-color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}

.hf-search-bar:hover {
    border-color: var(--border-strong);
}

/* Sort chips */

.hf-chip {
    padding: 6px 14px;
    border-radius: var(--radius-pill);
    background-color: var(--bg-search);
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 500;
    border-width: 1px;
    border-color: transparent;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard),
                transform var(--duration-fast) var(--ease-standard);
}

.hf-chip:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.hf-chip.selected {
    background-color: var(--primary-soft);
    color: var(--primary);
    border-color: var(--primary);
}

.hf-chip:active {
    transform: scale(0.97);
}
