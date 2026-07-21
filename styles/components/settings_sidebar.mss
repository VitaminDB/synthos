/* Левый сайдбар настроек. Грамматика зеркалит .chats-column с главной:
 * ширина 300px, белый фон, заголовок 26px/700, разделители и паддинги как
 * у списка чатов. Это обеспечивает визуальную преемственность. */

.settings-sidebar {
    width: 300px;
    height: 100%;
    background-color: var(--bg-chats);
    border-right-width: 1px;
    border-color: var(--border-soft);
    padding: 24px 8px 16px 8px;
    transition: background-color var(--duration-med) var(--ease-standard),
                border-color var(--duration-med) var(--ease-standard);
}

.settings-sidebar-title {
    color: var(--text);
    font-size: 26px;
    font-weight: 700;
    padding: 0 12px 20px 12px;
}

.settings-tab-item {
    background-color: transparent;
    cursor: pointer;
    padding: 12px;
    border-radius: 12px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.settings-tab-item:hover {
    background-color: var(--surface-hover);
}

.settings-tab-item.selected {
    background-color: var(--surface-selected);
}

.settings-tab-icon {
    color: var(--text-muted);
    icon-size: 22px;
    transition: color var(--duration-fast) var(--ease-standard);
}

.settings-tab-item.selected .settings-tab-icon {
    color: var(--primary);
}

.settings-tab-title {
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
    transition: color var(--duration-fast) var(--ease-standard);
}

.settings-tab-subtitle {
    color: var(--text-muted);
    font-size: 13px;
}

.settings-tab-item.selected .settings-tab-title {
    color: var(--primary-hover);
}
