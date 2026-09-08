/* Панель визарда в ленте чата (`pages::syn_chat::wizard`): вопрос
 * ассистента с кнопками-вариантами и/или полем ответа. Карточка-форма
 * стабильной ширины (до 640px), а не пузырёк: вопрос как markdown
 * (`.msg-bubble-md` + `.wizard-question-md`), варианты в строку с
 * переносом или столбиком, подвал под тонкой линией. */

.wizard-card {
    background-color: var(--surface-hover);
    border-width: 1px;
    border-color: var(--border);
    border-radius: 14px;
    padding: 14px 16px;
    max-width: 640px;
    box-shadow: 0 1px 2px rgba(17, 24, 39, 0.06);
}

.wizard-card-collapsed {
    border-radius: 12px;
    padding: 8px 12px;
}

/* Значок вопроса в мягком кружке — цвет акцента, без рамки-«тревоги». */
.wizard-icon-badge {
    width: 28px;
    height: 28px;
    border-radius: 14px;
    background-color: var(--primary-soft);
}

.wizard-icon {
    color: var(--primary);
    icon-size: 18px;
}

/* Вопрос выровнен по центру значка (28px против строки 15px). */
.wizard-question {
    padding-top: 3px;
}

/* Поверх `.msg-bubble-md`: чуть крупнее ответа ассистента — это
 * обращение к пользователю. Код, списки, цитаты — как в пузырьке. */
.msg-bubble-md.wizard-question-md {
    font-size: 15px;
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

/* Вариант одиночного выбора. В строке с переносом — «таблетка» не уже
 * 96px, чтобы короткие ответы не выглядели крошками; столбиком
 * (`.wizard-option-wide`) — на всю ширину карточки. */
Button.wizard-option {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border);
    border-radius: 10px;
    padding: 9px 16px;
    min-width: 96px;
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard);
}
Button.wizard-option:hover {
    border-color: var(--primary);
    background-color: var(--surface-selected);
}
Button.wizard-option.selected {
    border-color: var(--primary);
    background-color: var(--primary-soft);
    color: var(--primary);
    font-weight: 600;
}
Button.wizard-option-wide {
    width: 100%;
    padding: 10px 16px;
}

/* Множественный выбор — чипы-флажки. */
.wizard-chip {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border);
    color: var(--text);
    cursor: pointer;
}
.wizard-chip:hover {
    border-color: var(--primary);
}
.wizard-chip.selected {
    background-color: var(--primary);
    border-color: var(--primary);
    color: var(--on-primary);
}

.wizard-custom {
    min-width: 240px;
}

/* Подвал отделён тонкой линией; над ним, если есть таймер, — полоса
 * остатка времени на всю ширину карточки. */
.wizard-footer {
    border-top-width: 1px;
    border-top-color: var(--border-soft);
    padding-top: 10px;
}

.wizard-timer-bar {
    height: 3px;
    border-radius: 2px;
    background-color: var(--border);
    accent-color: var(--primary);
}

.wizard-footer-icon {
    color: var(--text-subtle);
    icon-size: 14px;
}

.wizard-footer-text {
    font-size: 11px;
    color: var(--text-subtle);
}

/* Последние секунды отсчёта. */
.wizard-footer-icon.urgent {
    color: var(--warning);
}
.wizard-footer-text.urgent {
    color: var(--warning);
    font-weight: 600;
}
