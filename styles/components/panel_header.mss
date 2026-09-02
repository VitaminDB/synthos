/* Заголовки панелей трёхпанельного каркаса (`components::panel_header`,
 * `components::workspace_frame`): у каждой колонки свой заголовок одной
 * высоты — тогглы у внешних краёв, идентичность и поиск в центре. */

.panel-header {
    height: 52px;
    padding: 0 8px 0 8px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.panel-header--center {
    background-color: var(--bg-shell);
}

.panel-header--side {
    background-color: var(--bg-panel);
}

.workspace-frame {
    height: 100%;
}

.workspace-body {
}

/* Заголовок боковой панели: иконка + текст. */
.panel-header-side-icon {
    icon-size: 18px;
    color: var(--text-muted);
}

.panel-header-side-title {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
}

.panel-header-left {
}

.panel-header-identity {
    padding: 0 4px 0 0;
}

/* Колонка «заголовок + подзаголовок» забирает остаток ширины. Это не
 * косметика: Row меряет не-flex детей с бесконечной шириной, и без
 * flex-grow текст никогда не узнаёт, что места мало — многоточие
 * (`max_lines`, `Elide::Middle`) не срабатывает, и длинный путь вылезает
 * на соседнюю панель. */
.panel-header-identity-info {
    flex-grow: 1;
}

/* Круглая подложка с иконкой страницы — той же высоты, что аватар чата,
 * чтобы заголовок не менял высоту между разделами. */
.panel-header-icon-bubble {
    width: 30px;
    height: 30px;
    border-radius: 10px;
    background-color: var(--primary-soft);
}

.panel-header-icon {
    icon-size: 18px;
    color: var(--primary);
}

.panel-header-title {
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
}

.panel-header-subtitle {
    font-size: 11px;
    color: var(--text-muted);
}

/* Обойма пилюли поиска: забирает место между идентичностью и кнопками, но
 * сама пилюля держит свои 380px и стоит по центру обоймы. */
.panel-header-center {
    padding: 0 12px 0 12px;
}

.panel-header-right {
}

.panel-header-action {
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border);
    background-color: var(--bg-shell);
    color: var(--text-muted);
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.panel-header-action:hover { background-color: var(--surface-hover); }

/* Архив чата — «опасное» действие в ряду: на hover краснеет. */
.panel-header-action-danger:hover {
    color: var(--error);
    border-color: var(--error);
}

.panel-header-action-divider {
    width: 1px;
    height: 22px;
    background-color: var(--border);
    margin: 0 4px 0 4px;
}

/* Тоггл боковой панели: приглушён, пока панель скрыта, акцентный — пока
 * открыта. */
.panel-header-toggle {
    width: 32px;
    height: 32px;
    icon-size: 18px;
    border-radius: 8px;
    background-color: transparent;
    color: var(--text-subtle);
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.panel-header-toggle:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.panel-header-toggle--on {
    color: var(--primary);
}

/* Распорка справа в заголовке чата без активного чата: держит пилюлю
 * поиска ровно там же, где она стоит с открытым чатом. */
.chat-header-actions-placeholder {
    width: 96px;
}

/* Inline-переименование (чат, граф). Просмотр: название + приглушённый
 * карандаш, проявляющийся на hover — подсказка, что имя кликабельно.
 * Правка: поле ровно в габаритах строки заголовка (шрифт/вес совпадают с
 * .panel-header-title, вертикальный паддинг съеден в 1px) — переключение
 * режимов не двигает ни подзаголовок, ни пилюлю поиска. */
/* Выравнивание по одной вертикали: у названия (просмотр), у текста в поле
 * правки (padding поля уважается TextField'ом) и у подзаголовка «Модель…»
 * общий левый отступ 6px. Отрицательные margin-компенсации не в ходу. */
.chat-header-title-wrap {
    cursor: pointer;
    border-radius: 8px;
    padding: 1px 6px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.chat-header-title-wrap:hover {
    background-color: var(--surface-hover);
}

.chat-header-title-pencil {
    icon-size: 13px;
    color: transparent;
    transition: color var(--duration-fast) var(--ease-standard);
}

.chat-header-title-wrap:hover .chat-header-title-pencil {
    color: var(--text-subtle);
}

.chat-header-title-wrap-editing {
    cursor: text;
    padding: 0;
    background-color: transparent;
}

.chat-header-title-wrap-editing:hover {
    background-color: transparent;
}

/* «Модель не загружена» — на той же вертикали, что буква Н названия. */
.chat-header-subtitle-wrap {
    padding-left: 6px;
}

.chat-header-title-edit {
    font-size: 15px;
    font-weight: 600;
    width: 292px;
    /* Дефолтная высота TextField — 40px: колонка «название + модель»
     * распухала, и подзаголовок уезжал под нижний край шапки. 26px — чуть
     * воздуха тексту, но всё ещё в габаритах строки. */
    height: 26px;
    /* Текст поля на тех же 6px, что название и подзаголовок. */
    padding: 0 6px;
    border-radius: 8px;
    background-color: var(--bg-search);
    border-width: 1.5px;
    border-color: var(--primary);
    color: var(--text);
}
