/* Правый сайдбар страницы «Чаты» — контейнер + таб-бар + customer details. */

.right-panel {
    /* Ширину задаёт SplitView (`right_split_ratio`), поэтому фиксированного
     * `width` нет; левая граница рисуется полоской дивайдера. */
    height: 100%;
    background-color: var(--bg-panel);
}

/* Обёртка TabBar в заголовке правой панели чата: фон и границу даёт
 * `.panel-header`, здесь только контекст для селекторов `Tab`. */
.right-panel-tabbar {
    background-color: transparent;
}

/* Сам элемент TabBar: делим ширину между табами поровну. Класс стоит
 * прямо на TabBar, а не на обёртке, чтобы read MSS гарантированно сработал. */
.right-panel-tabbar-inner {
    --tab-fill: equal;
}

/* Tab элементы внутри TabBar: равномерно делят ширину (flex-grow: 1),
 * чтобы не болтаться «слипшимся блоком» слева. */
.right-panel-tabbar Tab {
    flex-grow: 1;
    height: 44px;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-muted);
    background-color: transparent;
    border-color: transparent;
    /* Цвет индикатора читается из target_props текущего состояния. Ставим
     * accent-color в базовом правиле, чтобы и в non-selected fallback
     * цепочка была тематической. */
    accent-color: var(--primary);
    transition: color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}

.right-panel-tabbar Tab:hover {
    color: var(--text);
    background-color: var(--surface-hover);
}

.right-panel-tabbar Tab:selected {
    color: var(--primary);
    font-weight: 600;
}

.right-panel-body {
    flex-grow: 1;
    background-color: var(--bg-panel);
    /* Внутренний отступ всего тела панели: контент не лепится к TabBar
     * сверху и не касается границ по горизонтали. Промежуток между
     * карточками даётся gap-ом Column в details::view(). */
    padding-top: 12px;
    padding-bottom: 16px;
    padding-left: 12px;
    padding-right: 12px;
}

/* Customer details (таб 1) — перенос старых правил из customer_panel.mss */
.customer-data-title {
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
    padding: 20px 20px 8px 20px;
}

.customer-row {
    padding: 10px 20px 10px 20px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.customer-row:hover { background-color: var(--surface-hover); }

.customer-row-icon  { icon-size: 18px; color: var(--text-muted); }
.customer-row-value { color: var(--text); font-size: 14px; }
.customer-row-value.muted { color: var(--text-muted); }
