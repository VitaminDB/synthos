/* Страница «Поддержка» — шапка с статусом и live log view. */

/* ── Корневой shell ───────────────────────────────────────────────────── */
.support-shell {
    background-color: var(--bg-chat);
    height: 100%;
}

/* ── Верхняя панель ──────────────────────────────────────────────────── */
.support-header {
    background-color: var(--bg-shell);
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.support-header-icon-wrap {
    width: 44px;
    height: 44px;
    background-color: var(--primary-soft);
    border-radius: 14px;
}

.support-header-icon {
    icon-size: 22px;
    color: var(--primary);
}

.support-header-title {
    color: var(--text);
    font-size: 18px;
    font-weight: 600;
}

.support-endpoint-text {
    color: var(--text-muted);
    font-size: 12px;
    font-family: "monospace";
}

/* Статус-пилюля — используется общий набор классов .llama-pill* из llama_control.mss. */
.support-pill-slot {
    width: 112px;
    height: 28px;
}

.support-clear-btn {
    height: 36px;
    border-radius: 10px;
    background-color: var(--surface-hover);
    color: var(--text);
    font-size: 13px;
    border-width: 1px;
    border-color: var(--border);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.support-clear-btn:hover {
    background-color: var(--bg-search);
    border-color: var(--border-strong);
}

/* ── Тело лога ───────────────────────────────────────────────────────── */
.support-log-wrap {
    background-color: #0f1420;
}

.support-log-area {
    background-color: #0f1420;
}

.support-log-list {
    background-color: #0f1420;
}

.support-log-line {
    color: #D1D5DB;
    font-size: 12px;
    font-family: "monospace";
}

/* ── Empty state ─────────────────────────────────────────────────────── */
.support-empty-bubble {
    width: 72px;
    height: 72px;
    border-radius: 999px;
    background-color: rgba(255, 255, 255, 0.06);
}

.support-empty-icon {
    icon-size: 32px;
    color: #9CA3AF;
}

.support-empty-title {
    color: #E5E7EB;
    font-size: 16px;
    font-weight: 600;
}

.support-empty-subtitle {
    color: #9CA3AF;
    font-size: 13px;
    text-align: center;
}
