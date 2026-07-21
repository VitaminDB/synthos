/* Маркер autocompact-итерации в ленте чата.
 *
 * Свёрнутый — узкая капсула с иконкой compress, заголовком (#итерация,
 * число свёрнутых сообщений, токены before→after), временем и chevron'ом.
 * Развёрнутый — добавляет MarkdownView с summary (`.compaction-summary`)
 * и список свёрнутых сообщений (`.compaction-collapsed-item` — приглушённый
 * стиль поверх стандартных msg-bubble'ов). */

.compaction-marker {
    background-color: var(--surface-hover);
    border-width: 1px;
    border-color: var(--border-soft);
    border-radius: 10px;
    padding: 10px 14px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.compaction-marker:hover {
    background-color: var(--surface);
}

.compaction-icon {
    color: var(--primary);
    icon-size: 18px;
}

.compaction-title {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
}

.compaction-time {
    color: var(--text-muted);
    font-size: 12px;
}

.compaction-chevron {
    color: var(--text-muted);
    icon-size: 18px;
}

/* Контейнер развёрнутого тела — мягкая отбивка от шапки. */
.compaction-body {
    padding: 8px 4px 0 4px;
}

/* Сжатое summary рендерится через MarkdownView; даём ему slightly меньший
 * font-size и нейтральные --md-* цвета (как у msg-thinking-body) чтобы
 * визуально отличалось от обычного ответа ассистента. */
.compaction-summary {
    color: var(--text);
    font-size: 13px;
    --md-heading-color: var(--text);
    --md-link-color: var(--primary);
    --md-code-bg: var(--bg-shell);
    --md-code-color: var(--text);
    --md-code-block-bg: var(--bg-window);
    --md-code-block-color: var(--text);
    --md-quote-bg: var(--bg-shell);
    --md-quote-text-color: var(--text-muted);
    --md-quote-border-color: var(--border);
    --md-bullet-color: var(--text-muted);
    --md-hr-color: var(--border);
}

/* Каждое свёрнутое сообщение под маркером — приглушённое (opacity), чтобы
 * глаз сразу понимал «это уже свёрнуто». */
.compaction-collapsed-item {
    opacity: 0.7;
}
