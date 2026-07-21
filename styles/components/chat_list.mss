/* Левая колонка «Chats» — CRUD-слой.
 * Цветовая палитра, радиусы и spacing наследуются из `base/variables.mss`;
 * здесь — только специфика нового UI (кнопка «+», trailing-корзина,
 * пустое состояние). Все переходы — через var(--duration-*) / var(--ease-*). */

/* ── Header: «Chats» + кнопка «+» ────────────────────────────────────────── */

.chats-header-row {
    padding: 4px 16px 4px 20px;
}

/* Эта кнопка — ToolButton. У ToolButton класс ставится прямо на его бокс,
 * так что правила .chats-header-add применяются напрямую. */
ToolButton.chats-header-add {
    width: 32px;
    height: 32px;
    border-radius: 10px;
    background-color: var(--primary);
    color: var(--on-primary);
    icon-size: 20px;
    transition:
        background-color var(--duration-fast) var(--ease-standard),
        transform var(--duration-fast) var(--ease-standard);
}
ToolButton.chats-header-add:hover {
    background-color: var(--primary-hover);
    transform: scale(1.06);
}
ToolButton.chats-header-add:active {
    transform: scale(0.96);
}

/* ── Карточка чата (дополнения поверх .conversation-item) ────────────────── */

/* Контейнер всей колонки участвует в hover-селекторе карточки: без
 * явного правила «inherit» на дочерних элементах hover-эффект работает
 * через CSS-каскад. Стилистика .conversation-item / .conv-name /
 * .conv-preview уже описана в conversation_item.mss — здесь только
 * trailing-корзина. */

ToolButton.chat-item-trailing-delete {
    width: 28px;
    height: 28px;
    border-radius: 8px;
    color: var(--text-muted);
    icon-size: 18px;
    opacity: 0.0;
    transition:
        opacity var(--duration-fast) var(--ease-standard),
        background-color var(--duration-fast) var(--ease-standard),
        color var(--duration-fast) var(--ease-standard);
}

.conversation-item:hover ToolButton.chat-item-trailing-delete {
    opacity: 1.0;
}

ToolButton.chat-item-trailing-delete:hover {
    background-color: var(--primary-soft);
    color: var(--error);
}

/* ── Пустое состояние списка ─────────────────────────────────────────────── */

.chats-empty-hint {
    padding: 18px 20px 14px 20px;
}

.chats-empty-hint-text {
    color: var(--text-muted);
    font-size: 13px;
    font-weight: 500;
}

/* ── Header активного чата (action-кнопка «…») ───────────────────────────── */
/* `.chat-header-action` уже определён в chat_header.mss; оставляем как есть. */
