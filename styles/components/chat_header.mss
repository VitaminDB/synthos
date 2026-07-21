.chat-header {
    padding: 18px 24px 18px 24px;
    background-color: var(--bg-shell);
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.chat-header-name {
    color: var(--text);
    font-size: 18px;
    font-weight: 700;
}

.chat-header-email {
    color: var(--text-muted);
    font-size: 13px;
}

.chat-header-action {
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border);
    background-color: var(--bg-shell);
    color: var(--text-muted);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.chat-header-action:hover { background-color: var(--surface-hover); }

/* ── Индикатор использования контекста llama ──────────────────────────────── */
.chat-header-ctx {
    /* Пустая обёртка: пока активного слота нет, виджет даёт zero-size
     * и не влияет на макет. */
}
.chat-header-ctx-icon {
    icon-size: 16px;
    color: var(--text-muted);
}
.chat-header-ctx-bar-wrap {
    /* Узкий кадр фиксированной ширины, чтобы в хедере не «растягивался». */
    width: 120px;
}
.chat-header-ctx-bar {
    height: 6px;
    border-radius: 3px;
    accent-color: var(--primary);
    background-color: var(--border-soft);
    transition: accent-color var(--duration-med) var(--ease-standard);
}
/* Warn-цвет включается, когда заполнение контекста ≥ порога autocompact'а
 * (по умолчанию 85%). Сигнализирует «пора сжать», даёт пользователю
 * причину нажать соседнюю кнопку Compact. */
.chat-header-ctx-bar-warn {
    accent-color: var(--warning, #F59E0B);
}
.chat-header-ctx-text {
    font-size: 12px;
    color: var(--text-muted);
    font-weight: 500;
}
/* Compact-кнопка справа от индикатора: ToolButton 28×28, нейтральная, при
 * disabled — приглушённая (это даёт ToolButton по дефолту через MSS-rules
 * tool-button:disabled — здесь только класс-маркер для возможных
 * переопределений в темах). */
.chat-header-ctx-compact {
    color: var(--text-muted);
}
.chat-header-ctx-compact:hover {
    color: var(--primary);
}
