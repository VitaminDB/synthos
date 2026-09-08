/* Панель визарда в ленте чата (`pages::syn_chat::wizard`): вопрос
 * ассистента с кнопками-вариантами и/или полем ответа. Живёт как
 * карточка инструмента, но заметнее — это обращение к пользователю. */

.wizard-card {
    background-color: var(--surface-hover);
    border-width: 1px;
    border-color: var(--primary);
    border-radius: 12px;
    padding: 12px 14px;
    max-width: 90%;
}

.wizard-card-collapsed {
    border-color: var(--border);
    padding: 8px 12px;
}

.wizard-icon {
    color: var(--primary);
    icon-size: 18px;
}

.wizard-question {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
}

.wizard-question-collapsed {
    font-size: 13px;
    color: var(--text-muted);
}

.wizard-status {
    font-size: 11px;
    font-weight: 600;
    color: var(--primary);
}

/* Вариант одиночного выбора — кнопка на всю ширину карточки. */
Button.wizard-option {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border);
    border-radius: 8px;
    padding: 8px 12px;
    color: var(--text);
}
Button.wizard-option:hover {
    border-color: var(--primary);
    background-color: var(--surface-selected);
}
Button.wizard-option.selected {
    border-color: var(--primary);
    background-color: var(--surface-selected);
}

.wizard-chip.selected {
    background-color: var(--primary);
    color: var(--on-primary);
}

.wizard-footer-icon {
    color: var(--text-subtle);
    icon-size: 14px;
}

.wizard-footer-text {
    font-size: 11px;
    color: var(--text-subtle);
}
