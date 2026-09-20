/* ===========================================================================
 * Settings → Базы знаний (RAG / KB)
 *
 * Страница собрана из тех же кирпичей, что «Общие»: `.settings-section-title`
 * + `.settings-card` со строками `.settings-row` (иконка в `--primary-soft`,
 * заголовок, описание, контрол справа). Здесь — только своё: шапка коллекции,
 * чипы состояния, кнопки, строки документов и выдача пробного поиска. Цвета —
 * только переменные темы: страница обязана жить во всех оформлениях.
 * =========================================================================== */

.kb-page {
    background-color: transparent;
}

/* Даёт тексту внутри flex-строки ужиматься (elide), а не распирать строку. */
.kb-min0 {
    min-width: 0;
}

/* ── Пустое состояние ─────────────────────────────────────────────────── */

.kb-empty-bubble {
    width: 88px;
    height: 88px;
    border-radius: 999px;
    background-color: var(--primary-soft);
}

.kb-empty-icon {
    color: var(--primary);
    icon-size: 40px;
}

.kb-empty-title {
    font-size: 20px;
    font-weight: 700;
    color: var(--text);
}

/* Ширину ограничивает сам текст: обёртка-бокс вокруг него в свободной по
 * высоте колонке растягивалась на всю высоту страницы. */
.kb-empty-text {
    max-width: 460px;
    font-size: 14px;
    line-height: 21px;
    color: var(--text-muted);
    text-align: center;
}

/* ── Шапка коллекции ──────────────────────────────────────────────────── */

.kb-hero-badge {
    width: 56px;
    height: 56px;
    border-radius: 16px;
    background-color: var(--primary-soft);
}

.kb-hero-badge-icon {
    color: var(--primary);
    icon-size: 28px;
}

/* Имя — поле без рамки: выглядит заголовком, пока в него не щёлкнули. */
TextField.kb-hero-name {
    width: 100%;
    background-color: transparent;
    border-color: transparent;
    border-radius: 10px;
    padding: 6px 10px;
    font-size: 20px;
    font-weight: 700;
    color: var(--text);
}

TextField.kb-hero-name:hover {
    background-color: var(--bg-search);
}

TextField.kb-hero-name:focus {
    background-color: var(--bg-search);
    border-color: var(--primary);
}

.kb-stat-chip {
    padding: 4px 10px;
    border-radius: 999px;
    background-color: var(--bg-search);
    border-width: 1px;
    border-color: var(--border-soft);
}

.kb-stat-chip-icon {
    icon-size: 14px;
    color: var(--text-subtle);
}

.kb-stat-chip-label {
    font-size: 12px;
    color: var(--text-muted);
}

.kb-stat-chip-value {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
}

.kb-confirm-text {
    font-size: 13px;
    color: var(--text-muted);
}

/* ── Кнопки ───────────────────────────────────────────────────────────── */

.kb-btn {
    padding: 8px 14px;
    border-radius: 10px;
    background-color: transparent;
    border-width: 1px;
    border-color: var(--border-strong);
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard);
}

.kb-btn:hover {
    background-color: var(--surface-hover);
}

.kb-btn.kb-btn-primary {
    background-color: var(--primary);
    border-color: var(--primary);
    color: var(--on-primary);
    font-weight: 600;
}

.kb-btn.kb-btn-primary:hover {
    background-color: var(--primary-hover);
    border-color: var(--primary-hover);
}

.kb-btn.kb-btn-soft {
    background-color: var(--primary-soft);
    border-color: transparent;
    color: var(--primary);
    font-weight: 600;
}

.kb-btn.kb-btn-soft:hover {
    background-color: var(--primary);
    color: var(--on-primary);
}

.kb-btn.kb-btn-danger {
    background-color: var(--error);
    border-color: var(--error);
    color: var(--text-inverse);
    font-weight: 600;
}

.kb-btn.kb-btn-danger:hover {
    background-color: var(--error-hover);
    border-color: var(--error-hover);
}

/* «В чате» — пилюля-переключатель: выключена — контур, включена — акцент. */
.kb-chat-pill {
    padding: 8px 14px;
    border-radius: 999px;
    background-color: transparent;
    border-width: 1px;
    border-color: var(--border-strong);
    color: var(--text-muted);
    font-size: 13px;
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.kb-chat-pill:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.kb-chat-pill.on {
    background-color: var(--primary-soft);
    border-color: transparent;
    color: var(--primary);
    font-weight: 600;
}

.kb-chat-pill.on:hover {
    background-color: var(--primary-soft);
    color: var(--primary-hover);
}

.kb-icon-btn {
    color: var(--text-muted);
    background-color: transparent;
    border-radius: 8px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.kb-icon-btn:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.kb-icon-btn.danger:hover {
    color: var(--error);
}

/* ── Строки-кнопки «Источники» ────────────────────────────────────────── */

.kb-click-row {
    cursor: pointer;
}

.kb-row-action-icon {
    icon-size: 20px;
    color: var(--text-subtle);
}

.kb-progress-bar {
    height: 6px;
    border-radius: 3px;
    accent-color: var(--primary);
    background-color: var(--bg-search);
}

/* ── Документы ────────────────────────────────────────────────────────── */

TextField.kb-filter-field {
    padding: 6px 12px;
    font-size: 13px;
}

.kb-doc-title {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
}

.kb-doc-path {
    color: var(--text-muted);
    font-size: 12px;
}

.kb-doc-size {
    color: var(--text-subtle);
    font-size: 12px;
    font-family: monospace;
}

.kb-placeholder-icon {
    icon-size: 32px;
    color: var(--text-subtle);
}

.kb-placeholder-text {
    font-size: 13px;
    color: var(--text-muted);
    text-align: center;
}

/* ── Проверка поиска ──────────────────────────────────────────────────── */

TextField.kb-probe-field {
    width: 100%;
}

.kb-probe-row {
    border-top-width: 1px;
    border-color: var(--border-soft);
}

.kb-probe-note {
    font-size: 13px;
    color: var(--text-muted);
}

.kb-probe-note.error {
    color: var(--error);
}

.kb-probe-rank {
    width: 28px;
    height: 28px;
    border-radius: 999px;
    background-color: var(--primary-soft);
}

.kb-probe-rank-text {
    font-size: 12px;
    font-weight: 700;
    color: var(--primary);
}

.kb-probe-snippet {
    font-size: 13px;
    line-height: 19px;
    color: var(--text-muted);
}

.kb-badge {
    padding: 2px 8px;
    border-radius: 6px;
    background-color: var(--bg-search);
    border-width: 1px;
    border-color: var(--border-soft);
}

.kb-badge.accent {
    background-color: var(--primary-soft);
    border-color: transparent;
}

.kb-badge-text {
    font-size: 11px;
    color: var(--text-muted);
    font-family: monospace;
}

.kb-badge.accent .kb-badge-text {
    color: var(--primary);
}

/* ── Модели ───────────────────────────────────────────────────────────── */

.kb-desc-warn {
    color: var(--warning);
}

.kb-desc-error {
    color: var(--error);
}

.kb-state-chip {
    padding: 4px 10px;
    border-radius: 999px;
    background-color: var(--bg-search);
    border-width: 1px;
    border-color: var(--border-soft);
}

.kb-state-chip-text {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
}

.kb-state-chip.ready {
    border-color: var(--success);
}

.kb-state-chip.ready .kb-state-chip-text {
    color: var(--success);
}

.kb-state-chip.busy {
    border-color: var(--primary);
}

.kb-state-chip.busy .kb-state-chip-text {
    color: var(--primary);
}

/* ── Правая панель: список коллекций (строки — от .skill-list-*) ──────── */

.kb-panel {
    background-color: transparent;
}

/* Коллекция подключена к чату — иконка залита акцентом. */
.kb-coll-in-chat {
    background-color: var(--primary);
}

.kb-coll-in-chat-icon {
    color: var(--on-primary);
}

.kb-coll-chat.on {
    color: var(--primary);
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
