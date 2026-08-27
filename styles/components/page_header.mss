/* Общая шапка страницы (`components::page_header`): одна строка на всю
 * ширину контента — тогглы панелей по краям, идентичность слева, пилюля
 * глобального поиска по центру, действия справа. */

.page-header {
    background-color: var(--bg-shell);
    border-bottom-width: 1px;
    border-color: var(--border-soft);
    padding: 8px 12px 8px 12px;
}

.page-header-left {
    /* Левый кластер не растягивается: пилюля в центре забирает всё
     * свободное место через `.page-header-center.grow`. */
}

.page-header-identity {
    padding: 0 4px 0 0;
}

/* Круглая подложка с иконкой страницы — той же высоты, что аватар чата,
 * чтобы шапка не меняла высоту между разделами. */
.page-header-icon-bubble {
    width: 30px;
    height: 30px;
    border-radius: 10px;
    background-color: var(--primary-soft);
}

.page-header-icon {
    icon-size: 18px;
    color: var(--primary);
}

.page-header-title {
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
}

.page-header-subtitle {
    font-size: 11px;
    color: var(--text-muted);
}

/* Обойма пилюли поиска: забирает место между идентичностью и кнопками, но
 * сама пилюля держит свои 380px и стоит по центру обоймы. */
.page-header-center {
    padding: 0 12px 0 12px;
}

.page-header-right {
}

.page-header-action {
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border);
    background-color: var(--bg-shell);
    color: var(--text-muted);
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.page-header-action:hover { background-color: var(--surface-hover); }

/* Архив чата / удаление — единственные «опасные» действия в ряду:
 * на hover краснеют, чтобы не перепутать с «очистить». */
.page-header-action-danger:hover {
    color: var(--error);
    border-color: var(--error);
}

.page-header-action-divider {
    width: 1px;
    height: 22px;
    background-color: var(--border);
    margin: 0 4px 0 4px;
}

/* Тоггл боковой панели: приглушён, пока панель скрыта, акцентный — пока
 * открыта. */
.page-header-toggle {
    width: 32px;
    height: 32px;
    icon-size: 18px;
    border-radius: 8px;
    background-color: transparent;
    color: var(--text-subtle);
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.page-header-toggle:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.page-header-toggle--on {
    color: var(--primary);
}

/* Распорка справа в шапке чата без активного чата: держит пилюлю поиска
 * ровно там же, где она стоит с открытым чатом. */
.chat-header-actions-placeholder {
    width: 96px;
}

/* Inline-переименование (чат, граф): поле той же высоты, что заголовок. */
.chat-header-title-wrap {
    cursor: pointer;
}

.chat-header-title-edit {
    font-size: 15px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 6px;
    background-color: var(--bg-search);
    border-width: 1px;
    border-color: var(--primary);
    color: var(--text);
}
