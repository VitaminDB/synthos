/* Таб «Ламма контроль» — заголовок, статус-пилюля, дропдаун, сводка, кнопки. */

/* ── Заголовок ─────────────────────────────────────────────────────────── */
.llama-header-icon-wrap {
    width: 36px;
    height: 36px;
    background-color: var(--primary-soft);
    border-radius: 12px;
}

.llama-header-icon {
    icon-size: 20px;
    color: var(--primary);
}

.llama-header-title {
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
}

.llama-header-subtitle {
    color: var(--text-subtle);
    font-size: 12px;
}

/* ── Статус-пилюля ─────────────────────────────────────────────────────── */
.llama-status-pill-slot {
    width: 112px;
    height: 28px;
}

/* Базовая пилюля: форма, размеры, padding. */
.llama-pill {
    width: 112px;
    height: 28px;
    border-radius: var(--radius-pill);
    padding: 0 12px 0 12px;
    transition: background-color var(--duration-med) var(--ease-standard);
}

.llama-pill-text {
    font-size: 12px;
    font-weight: 500;
}

/* Состояния — каждое отдельным классом, чтобы не зависеть от compound-селекторов. */
.llama-pill-stopped {
    background-color: var(--surface-hover);
}
.llama-pill-stopped .llama-pill-text {
    color: var(--text-muted);
}

.llama-pill-starting {
    background-color: #FEF3C7;
    animation: llama-pulse 1200ms ease-in-out infinite;
}
.llama-pill-starting .llama-pill-text {
    color: #92400E;
}

.llama-pill-running {
    background-color: #DCFCE7;
}
.llama-pill-running .llama-pill-text {
    color: #166534;
    font-weight: 600;
}

.llama-pill-error {
    background-color: #FEE2E2;
}
.llama-pill-error .llama-pill-text {
    color: #991B1B;
    font-weight: 600;
}

@keyframes llama-pulse {
    0%   { opacity: 1.0; }
    50%  { opacity: 0.55; }
    100% { opacity: 1.0; }
}

/* ── Дропдаун выбора пресета ──────────────────────────────────────────── */
.llama-picker-wrap {
    padding: 0 0 0 0;
}

.llama-picker-dropdown {
    width: 100%;
    height: 44px;
    font-size: 13px;
    border-radius: 10px;
    background-color: var(--bg-shell);
    color: var(--text);
    border-color: var(--border);
    accent-color: var(--primary);
    /* Тёма для всплывающего списка — читается элементом Dropdown в
     * apply_computed_style через кастомные --popup-* переменные. */
    --popup-background: var(--bg-shell);
    --popup-color: var(--text);
    --popup-border: var(--border);
    --popup-accent: var(--primary);
    --popup-hover-background: var(--surface-hover);
    --popup-hover-color: var(--text);
    --popup-selected-background: var(--surface-selected);
    --popup-selected-color: var(--primary);
}

.llama-picker-empty {
    color: var(--text-subtle);
    font-size: 13px;
    padding: 12px 0 12px 0;
}

/* ── Сводка выбранного пресета ────────────────────────────────────────── */
.llama-summary {
    background-color: var(--surface-hover);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.llama-summary:hover {
    background-color: var(--bg-search);
}

.llama-summary-hint {
    color: var(--text-subtle);
    font-size: 12px;
}

.llama-summary-icon {
    icon-size: 16px;
    color: var(--text-muted);
}

.llama-summary-file {
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
}

.llama-summary-meta {
    color: var(--text-muted);
    font-size: 12px;
}

.llama-summary-dot {
    width: 3px;
    height: 3px;
    border-radius: 999px;
    background-color: var(--text-subtle);
}

/* ── Кнопки старт/стоп ───────────────────────────────────────────────── */
.llama-controls {
    padding: 0;
}

.llama-btn-start {
    height: 44px;
    border-radius: 12px;
    background-color: var(--primary);
    color: var(--on-primary);
    font-size: 13px;
    font-weight: 600;
    transition: background-color var(--duration-fast) var(--ease-standard),
                opacity var(--duration-fast) var(--ease-standard);
}

.llama-btn-start:hover {
    background-color: var(--primary-hover);
}

.llama-btn-start:disabled {
    background-color: var(--border);
    color: var(--text-subtle);
    cursor: default;
}

.llama-btn-stop {
    height: 44px;
    border-radius: 12px;
    background-color: var(--surface-hover);
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
    border-width: 1px;
    border-color: var(--border);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.llama-btn-stop:hover {
    background-color: var(--bg-search);
    border-color: var(--border-strong);
}

.llama-btn-stop:disabled {
    background-color: transparent;
    color: var(--text-subtle);
    border-color: var(--border-soft);
    cursor: default;
}

.llama-btn-console {
    height: 36px;
    border-radius: 10px;
    background-color: transparent;
    color: var(--text-muted);
    font-size: 12px;
    border-width: 1px;
    border-color: var(--border-soft);
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.llama-btn-console:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}
