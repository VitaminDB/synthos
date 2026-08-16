.chats-column {
    background-color: var(--bg-chats);
    /* Ширина задаётся drag-разделителем `.syn-chat-h-split` (ratio живёт в
     * `SynChatCtx.left_split_ratio` и persist'ится в config.json), поэтому
     * фиксированного `width` здесь нет — колонка занимает весь свой слот.
     * Border тоже убран: его роль играет визуальная полоска дивайдера. */
    /* Сверху уменьшено: header_row уже даёт свой padding-top=4px,
     * двойной отступ (20+4) визуально разрывал заголовок
     * от верхнего края панели. */
    padding: 8px 0 16px 0;
}

.chats-column-title {
    color: var(--text);
    font-size: 26px;
    font-weight: 700;
    /* Горизонтальный отступ задаётся `.chats-header-row` (padding-left=20px).
     * Здесь — только нижний отступ перед фильтр-чипом. */
    padding-bottom: 6px;
}

.chats-filter-wrap {
    padding: 0 20px 0 20px;
}

.filter-chip {
    background-color: var(--bg-shell);
    border-width: 1px;
    border-color: var(--border);
    border-radius: var(--radius-pill);
    padding: 6px 14px 6px 14px;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.filter-chip:hover { background-color: var(--surface-hover); }

.filter-chip-icon  { icon-size: 16px; color: var(--text-muted); }
.filter-chip-text  { color: var(--text); font-size: 13px; font-weight: 500; }
.filter-chip-dot   { width: 6px; height: 6px; border-radius: 50%; background-color: var(--primary); }

.chats-group-wrap {
    padding: 8px 20px 0 20px;
}

/* Контейнер ListView чатов — горизонтальный воздух между карточками
 * и боковыми границами панели. Сами карточки `.conversation-item` имеют
 * собственный внутренний padding, поэтому здесь — только тонкая «дорожка». */
.chats-list-wrap {
    padding: 0 5px 0 5px;
}

.chats-group-label {
    color: var(--text-muted);
    font-size: 13px;
    font-weight: 600;
}

.chats-group-chev { icon-size: 16px; color: var(--text-muted); }

.chats-section-divider {
    height: 1px;
    background-color: var(--border-soft);
    padding: 0 20px 0 20px;
}

.chats-footer-row {
    padding: 10px 20px 10px 20px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.chats-footer-row:hover {
    background-color: var(--surface-hover);
}

.chats-footer-icon  { icon-size: 20px; color: var(--text-muted); }
.chats-footer-label { color: var(--text); font-size: 14px; }
.chats-footer-count { color: var(--text-muted); font-size: 13px; }
