/* Левая панель: список карточек найденных моделей. */

.hf-list-panel {
    background-color: var(--bg-shell);
    padding: var(--spacing-md);
    height: 100%;
}

.hf-list-scroll {
    height: 100%;
}

/* Чипы сортировки над списком. */
.hf-list-chips {
    padding: 0 0 2px 0;
}

.hf-list-body {
    height: 100%;
}

/* Заголовок секции в списке («Качается» / «Модели»). */
.hf-list-section {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    padding: 4px 2px 0 2px;
}

/* Карточка модели */

.hf-card {
    padding: var(--spacing-md);
    border-radius: var(--radius-panel);
    background-color: var(--bg-shell);
    border-width: 1px;
    border-color: var(--border-soft);
    transition: transform var(--duration-fast) var(--ease-standard),
                box-shadow var(--duration-med) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}

.hf-card:hover {
    transform: translateY(-2px);
    border-color: var(--border-strong);
    box-shadow: 0 8px 18px rgba(24, 24, 43, 0.08);
}

.hf-card.selected {
    background-color: var(--surface-selected);
    border-color: var(--primary);
}

.hf-card-avatar {
    icon-size: 28px;
    color: var(--primary);
    padding: 6px;
    background-color: var(--primary-soft);
    border-radius: var(--radius-pill);
}

/* Letter-avatar автора. 36×36 круглая, цвет задаётся inline (детерминирован
 * хэшем имени), буква по центру на белом. HF не отдаёт публичный avatar
 * без auth, поэтому стилизуем сами — выглядит как у GitHub/Slack. */
.hf-card-avatar-letter {
    width: 36px;
    height: 36px;
    border-radius: 18px;
    overflow: hidden;
}

.hf-card-avatar-letter-text {
    font-size: 16px;
    font-weight: 600;
    color: #FFFFFF;
}

.hf-card-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    line-clamp: 1;
}

.hf-card-author {
    font-size: 12px;
    color: var(--text-muted);
}

.hf-card-stats {
    padding-top: var(--spacing-xs);
}

.hf-stat-icon {
    icon-size: 14px;
    color: var(--text-subtle);
}

.hf-stat-text {
    font-size: 11px;
    color: var(--text-muted);
    font-weight: 500;
}

.hf-stat-time {
    font-size: 11px;
    color: var(--text-subtle);
    margin-left: var(--spacing-sm);
}

.hf-card-pipeline-empty { /* пустая плашка — не занимает пространство */
    height: 0;
}

.hf-card-pipeline-chip {
    padding: 2px 10px;
    background-color: var(--bg-search);
    border-radius: var(--radius-pill);
    border-width: 1px;
    border-color: var(--border-soft);
    margin-top: var(--spacing-xs);
}

.hf-card-pipeline-text {
    font-size: 11px;
    color: var(--text-muted);
    font-weight: 500;
}

/* Skeleton-загрузка карточек */

.hf-card.loading {
    height: 88px;
    background-color: var(--surface-hover);
    border-color: var(--border-soft);
}

.hf-card.loading:hover {
    transform: none;
    box-shadow: none;
}

/* Error placeholder */

.hf-list-error {
    padding: var(--spacing-xl);
    background-color: var(--bg-shell);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border);
    max-width: 360px;
}

.hf-list-error-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
}

.hf-list-error-msg {
    font-size: 12px;
    color: var(--text-muted);
    text-align: center;
}

.hf-retry-btn {
    padding: 8px 18px;
    background-color: var(--primary);
    color: var(--on-primary);
    border-radius: var(--radius-pill);
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.hf-retry-btn:hover {
    background-color: var(--primary-hover);
}

.hf-list-empty {
    color: var(--text-subtle);
    font-size: 13px;
}
