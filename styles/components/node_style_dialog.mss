/* Portal-диалог «Цвет ноды» (ColorPicker для NodeStyle.tint).
 * Структура — карточка-панель с заголовком, подсказкой, ColorPicker
 * и тремя действиями: Сбросить / Отмена / Применить.
 *
 * Базовая стилистика match'ит остальные диалоги synthos
 * (`kb-url-dialog-*`, `skill-dialog-*`) — общая палитра, отступы,
 * радиусы. Иконка в заголовке — primary-accent (палитра «Material»). */

.node-tint-dialog-card {
    background-color: var(--bg-panel);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 24px 24px 20px 24px;
    min-width: 360px;
    max-width: 420px;
}

.node-tint-dialog-icon {
    icon-size: 22px;
    color: var(--primary);
}

.node-tint-dialog-title {
    color: var(--text);
    font-size: 18px;
    font-weight: 600;
}

.node-tint-dialog-hint {
    color: var(--text-muted);
    font-size: 13px;
    line-height: 1.45;
}

/* Сам ColorPicker: без класса он рисует попап белым по светлым
 * умолчаниям виджета — в тёмной теме карточка и палитра расходились. */
.node-tint-dialog-picker {
    background-color: var(--bg-panel);
    color: var(--text);
    border-color: var(--border);
    accent-color: var(--primary);
    border-radius: 10px;
    height: 32px;
    font-size: 13px;
    --popup-background: var(--bg-shell);
    --popup-color: var(--text);
    --popup-border: var(--border);
}

.node-tint-dialog-btn-primary {
    background-color: var(--primary);
    color: var(--on-primary);
    border-radius: var(--radius-panel);
    padding: 8px 16px;
    font-size: 13px;
    font-weight: 600;
    transition: background-color 140ms ease-out;
}
.node-tint-dialog-btn-primary:hover {
    background-color: var(--primary-hover);
}

.node-tint-dialog-btn-secondary {
    background-color: transparent;
    color: var(--text);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 8px 16px;
    font-size: 13px;
    transition: background-color 140ms ease-out, border-color 140ms ease-out;
}
.node-tint-dialog-btn-secondary:hover {
    background-color: var(--surface-hover);
    border-color: var(--border-strong);
}
