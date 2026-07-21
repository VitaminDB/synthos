/* Cache-dir prompt — модальный диалог выбора каталога для скачивания. */

.hf-dialog-empty { /* placeholder когда Portal закрыт */
    width: 0;
    height: 0;
}

.hf-dialog-card {
    padding: var(--spacing-xl);
    background-color: var(--bg-shell);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    box-shadow: var(--shadow-shell);
    min-width: 460px;
    max-width: 560px;
}

.hf-dialog-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
}

.hf-dialog-hint {
    font-size: 13px;
    color: var(--text-muted);
    line-height: 1.5;
}

.hf-dialog-btn-primary {
    padding: 8px 18px;
    background-color: var(--primary);
    color: var(--on-primary);
    border-radius: var(--radius-pill);
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.hf-dialog-btn-primary:hover {
    background-color: var(--primary-hover);
}

.hf-dialog-btn-secondary {
    padding: 8px 18px;
    background-color: var(--bg-search);
    color: var(--text);
    border-radius: var(--radius-pill);
    font-weight: 500;
    border-width: 1px;
    border-color: var(--border-soft);
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard);
}

.hf-dialog-btn-secondary:hover {
    background-color: var(--surface-hover);
    border-color: var(--border-strong);
}
