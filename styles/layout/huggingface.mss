/* Layout страницы HuggingFace: page-wrapper, body, split. */

.hf-page {
    background-color: var(--bg-window);
    width: 100%;
    height: 100%;
}

.hf-body {
    background-color: var(--bg-shell);
    border-radius: var(--radius-panel);
    margin: var(--spacing-md);
}

/* SplitView читает свой собственный MSS-контекст: `border-color` — цвет
 * полосы дивайдера в покое, `accent-color` — на hover/drag. Без этих
 * правил используются хардкоды `#E5E7EB` / `#3B82F6`, что не подхватывает
 * тему. */
.hf-split {
    height: 100%;
    border-color: var(--border-soft);
    accent-color: var(--primary);
    color: var(--text-subtle);
    divider-thickness: 1px;
}
