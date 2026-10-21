/* ─────────────────────── Страница «Обучение» ───────────────────────
 * Дообучение моделей адаптерами LoRA через службу нейросетей: левая
 * панель (задачи, датасеты, адаптеры), центр (обзор, форма, ход задачи,
 * редактор датасета, адаптер). Цвета — токены `styles/base/variables.mss`.
 * ───────────────────────────────────────────────────────────────── */

.training-page {
    background-color: var(--bg-shell);
}

.training-h-split {
    background-color: var(--bg-shell);
    border-color: var(--border-soft);
    accent-color: var(--primary);
    divider-thickness: 1px;
}

.training-center {
    background-color: var(--bg-shell);
}

.training-center-body {
    padding: 22px 28px 28px 28px;
    width: 100%;
    max-width: 980px;
}

/* ─── Левая панель ─────────────────────────────────────────────── */

.training-left {
    background-color: var(--bg-panel);
}

.training-left-body {
    padding: 8px 10px 16px 10px;
}

.training-left-section-row {
    padding: 14px 6px 4px 8px;
}

.training-left-section {
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.6px;
    text-transform: uppercase;
}

.training-left-empty {
    color: var(--text-subtle);
    font-size: 12px;
    padding: 4px 8px 6px 8px;
}

.training-item {
    padding: 8px 10px 8px 10px;
    border-radius: 10px;
    background-color: transparent;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.training-item:hover {
    background-color: var(--surface-hover);
}

.training-item.training-item-selected {
    background-color: var(--surface-selected);
}

.training-item-icon {
    font-size: 18px;
    color: var(--text-muted);
}

.training-item-selected .training-item-icon {
    color: var(--primary);
}

.training-item-title {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
}

.training-item-line {
    color: var(--text-muted);
    font-size: 11px;
}

.training-item-progress {
    height: 3px;
    border-radius: 2px;
    accent-color: var(--primary);
    background-color: var(--border-soft);
    margin: 4px 0 0 0;
}

/* ─── Карточки центра ──────────────────────────────────────────── */

.training-card {
    padding: 16px 18px 16px 18px;
    border-radius: var(--radius-panel);
    background-color: var(--bg-search);
    border-width: 1px;
    border-color: var(--border-soft);
}

.training-card-title {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
}

.training-section-title {
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.5px;
    text-transform: uppercase;
    padding: 6px 2px 0 2px;
}

.training-lead {
    color: var(--text);
    font-size: 14px;
    line-height: 1.5;
}

.training-hint {
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.45;
}

.training-warning {
    color: var(--warning);
    font-size: 12px;
}

.training-step-num {
    width: 30px;
    height: 30px;
    border-radius: 15px;
    background-color: var(--primary-soft);
}

.training-step-num-text {
    color: var(--primary);
    font-size: 14px;
    font-weight: 700;
}

/* ─── Форма ────────────────────────────────────────────────────── */

.training-form {
    padding: 4px 18px 4px 18px;
}

.training-field {
    padding: 12px 0 12px 0;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.training-field-label {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
}

.training-field-hint {
    color: var(--text-muted);
    font-size: 11px;
    line-height: 1.4;
}

.training-check-progress {
    width: 160px;
    height: 4px;
    accent-color: var(--primary);
    background-color: var(--border-soft);
}

/* ─── Кнопки ───────────────────────────────────────────────────── */

.training-btn {
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

.training-btn:hover {
    background-color: var(--surface-hover);
}

.training-btn.training-btn-primary {
    background-color: var(--primary);
    border-color: var(--primary);
    color: var(--on-primary);
    font-weight: 600;
}

.training-btn.training-btn-primary:hover {
    background-color: var(--primary-hover);
    border-color: var(--primary-hover);
}

.training-btn.training-btn-danger {
    border-color: var(--error);
    color: var(--error);
}

.training-btn.training-btn-danger:hover {
    background-color: var(--error);
    color: var(--text-inverse);
}

.training-btn.training-btn-small {
    padding: 5px 10px;
    font-size: 12px;
}

.training-icon-btn {
    width: 30px;
    height: 30px;
    border-radius: 8px;
    color: var(--text-muted);
    background-color: transparent;
}

.training-icon-btn:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

/* ─── Ход задачи ───────────────────────────────────────────────── */

.training-progress {
    height: 6px;
    border-radius: 3px;
    accent-color: var(--primary);
    background-color: var(--border-soft);
}

.training-badge {
    padding: 3px 10px;
    border-radius: 999px;
    background-color: var(--border-soft);
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 600;
}

.training-badge.training-badge-run {
    background-color: var(--primary-soft);
    color: var(--primary);
}

.training-badge.training-badge-ok {
    background-color: var(--diff-added-bg);
    color: var(--success);
}

.training-badge.training-badge-error {
    background-color: var(--diff-removed-bg);
    color: var(--error);
}

.training-stat-value {
    color: var(--text);
    font-size: 18px;
    font-weight: 700;
}

.training-stat-label {
    color: var(--text-muted);
    font-size: 11px;
}

.training-chart {
    width: 100%;
    color: var(--text-muted);
    accent-color: var(--primary);
}

.training-kv-label {
    width: 150px;
    color: var(--text-muted);
    font-size: 12px;
}

.training-kv-value {
    color: var(--text);
    font-size: 12px;
}

/* ─── Ошибки и пустые состояния ────────────────────────────────── */

.training-error-bar {
    padding: 10px 16px 10px 16px;
    background-color: var(--diff-removed-bg);
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.training-error-icon {
    color: var(--error);
    font-size: 18px;
}

.training-error-text {
    color: var(--error);
    font-size: 12px;
}

.training-empty-icon {
    color: var(--text-subtle);
    font-size: 40px;
}

.training-empty-title {
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
}

/* ─── Редактор датасета ────────────────────────────────────────── */

.training-example-title {
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 700;
}

.training-turn {
    padding: 8px 8px 8px 10px;
    border-radius: 10px;
    background-color: var(--bg-shell);
    border-left-width: 3px;
    border-color: var(--border-strong);
}

.training-turn.training-turn-user {
    border-color: var(--text-subtle);
}

.training-turn.training-turn-assistant {
    border-color: var(--primary);
}

.training-turn.training-turn-system {
    border-color: var(--warning);
}

.training-turn-edit {
    min-height: 34px;
    padding: 6px 8px;
    border-radius: 8px;
    background-color: var(--bg-search);
    color: var(--text);
    font-size: 13px;
}

.training-pager {
    padding: 4px 0 0 0;
}

/* ─── Карточка «Адаптеры LoRA» в чате ──────────────────────────── */

.lora-chat-body {
}

.lora-chat-row {
    padding: 8px 0 4px 0;
    border-top-width: 1px;
    border-color: var(--border-soft);
}

/* Выключенный переключатель на карточке: дорожка темнее фона карточки
 * (общий `Toggle` красится в --bg-search — цвет самой карточки). */
.training-toggle {
    background-color: var(--border-strong);
}
