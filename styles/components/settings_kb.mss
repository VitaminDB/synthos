/* ===========================================================================
 * Settings → Базы знаний (RAG / KB)
 *
 * Стиль повторяет audio_models / models — общая «карточная» сетка с белыми
 * (тёмно-серыми) панелями и outlines. Доп. стили — для kb-специфичных
 * элементов: progress bar индексации, чип «активна в чате», meta-line.
 * =========================================================================== */

.kb-page {
    background: var(--bg-shell, #f7f7f9);
}

.kb-meta-line {
    color: var(--text-muted, #6b7280);
    font-size: 12px;
    line-height: 18px;
}

.kb-embedder-status {
    color: var(--text, #1f2937);
    font-weight: 500;
    font-size: 14px;
}

.kb-embedder-hint {
    color: var(--text-subtle, #9ca3af);
    font-size: 12px;
    line-height: 16px;
}

.kb-embedder-btn {
    padding: 8px 14px;
    border-radius: 8px;
    background: var(--bg-panel, #ffffff);
    border: 1px solid var(--border, #e5e7eb);
    color: var(--text, #1f2937);
    transition: background 120ms ease, border-color 120ms ease;
}
.kb-embedder-btn:hover {
    background: var(--surface-hover, #f3f4f6);
}
.kb-embedder-btn.primary {
    background: var(--primary, #3B82F6);
    border-color: var(--primary, #3B82F6);
    color: #ffffff;
}
.kb-embedder-btn.primary:hover {
    background: var(--primary-hover, #2563EB);
}

.kb-source-btn {
    padding: 8px 12px;
    border-radius: 8px;
    background: var(--bg-panel, #ffffff);
    border: 1px solid var(--border, #e5e7eb);
    color: var(--text, #1f2937);
    transition: background 120ms ease;
}
.kb-source-btn:hover {
    background: var(--surface-hover, #f3f4f6);
}

.kb-progress-card {
    background: var(--bg-panel, #ffffff);
    border-color: var(--primary, #3B82F6);
}
.kb-progress-bar {
    height: 6px;
    border-radius: 3px;
    accent-color: var(--primary, #3B82F6);
    background-color: var(--border, #e5e7eb);
}
.kb-progress-line {
    color: var(--text-muted, #6b7280);
    font-size: 12px;
}

.kb-doc-row {
    background: var(--bg-panel, #ffffff);
    border: 1px solid var(--border-soft, #eef0f2);
    border-radius: 8px;
    transition: border-color 120ms ease;
}
.kb-doc-row:hover {
    border-color: var(--border, #e5e7eb);
}
.kb-doc-line {
    color: var(--text, #1f2937);
    font-size: 12px;
    line-height: 17px;
}

/* Правая панель — список коллекций */
.kb-panel { /* fallback к skills-panel/models-panel — наследуем layout */ }

.kb-coll-row {
    background: var(--bg-panel, #ffffff);
    border: 1px solid var(--border-soft, #eef0f2);
    border-radius: 10px;
    transition: border-color 120ms ease, background 120ms ease;
}
.kb-coll-row:hover {
    border-color: var(--border, #e5e7eb);
}
.kb-coll-row.selected {
    border-color: var(--primary, #3B82F6);
    background: var(--primary-soft, #eff6ff);
}
.kb-coll-title {
    color: var(--text, #1f2937);
    font-weight: 500;
    font-size: 14px;
}
.kb-coll-subtitle {
    color: var(--text-muted, #6b7280);
    font-size: 11px;
    line-height: 14px;
}
.kb-coll-toggle {
    padding: 4px 10px;
    font-size: 11px;
    border-radius: 6px;
    background: var(--bg-shell, #f7f7f9);
    border: 1px solid var(--border, #e5e7eb);
    color: var(--text-muted, #6b7280);
    transition: background 120ms ease;
}
.kb-coll-toggle:hover {
    background: var(--surface-hover, #f3f4f6);
}

/* Snackbar глобальный — общий для KB и других приложений (до этапа полного
 * рефакторинга code_editor::notice). Pill bottom-center, плавный fade. */
.snackbar-overlay {
    /* StackOverlay через Stack — занимает весь экран, но pointer-events: none
       позволяет кликам проходить мимо. */
    background: transparent;
}
.snackbar-pill {
    background: rgba(31, 41, 55, 0.92);  /* gray-800 ~ */
    border-radius: 10px;
    padding: 12px 18px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
    transition: opacity 200ms ease;
}
.snackbar-text {
    color: #ffffff;
    font-size: 13px;
    line-height: 18px;
}
.snackbar-hidden {
    /* Реактивная пустая ветка — нулевой размер */
    background: transparent;
}

/* ===========================================================================
 * URL prompt диалог (knowledge_base::url_dialog).
 * Геометрия и кнопки взяты из `.skill-dialog-*` (см. settings_skills.mss);
 * здесь — лишь свои title/hint/error и сама карточка с другой шириной.
 * =========================================================================== */
.kb-url-dialog-card {
    background-color: var(--bg-panel);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 24px 24px 20px 24px;
    min-width: 460px;
    max-width: 640px;
}

.kb-url-dialog-title {
    color: var(--text);
    font-size: 18px;
    font-weight: 600;
}

.kb-url-dialog-hint {
    color: var(--text-muted);
    font-size: 13px;
    line-height: 18px;
}

.kb-url-dialog-error {
    color: var(--error);
    font-size: 12px;
    line-height: 16px;
    /* Плавное появление при появлении/обновлении сообщения. */
    transition: color var(--duration-fast) var(--ease-standard);
}

.kb-url-dialog-empty {
    background: transparent;
}
