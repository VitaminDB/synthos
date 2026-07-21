/* ───────────────────────── Global TabBar / Tab style ──────────────────
 * Единая стилистика всех `syngui::TabBar` и `syngui::Tab` в приложении —
 * Templates panel, Settings, любые будущие страницы. Стилизация
 * Zed/VSCode-like: тонкий бар, подчёркивающий accent-индикатор под
 * активной вкладкой, плавные transitions на color/bg.
 *
 * Element-type селекторы `tab-bar` и `tab` цепляют все экземпляры
 * виджетов — никаких локальных классов писать не нужно. При необходимости
 * перебить точечно — используй .my-class на конкретном TabBar'е (правило
 * с классом будет иметь больший вес).
 *
 * Tab-Element читает свойства:
 *   color / background-color / border-color / accent-color / font-size /
 *   font-weight / border-radius / --tab-indicator-height /
 *   --tab-indicator-inset.
 * Подсветка `tab:selected` идёт через `accent-color: var(--primary)`
 * (см. `widgets/navigation/tab.rs::build_display_list` — primary берётся
 * из target-props selected).
 * ─────────────────────────────────────────────────────────────────── */

/* Element-type селекторы в syngui case-sensitive — имя должно совпадать с
 * `Element::element_type_name()` дословно. У syngui::TabBar это `TabBar`,
 * у syngui::Tab — `Tab` (см. widgets/navigation/{tab,tab_bar}.rs). */

TabBar {
    background-color: transparent;
    /* Один непрерывный border-bottom через всю ширину TabBar'а — линия
     * не разрывается между Tab'ами и за ними. Tab сам не рисует
     * собственный bottom-border (border-color transparent ниже). */
    border-bottom-width: 1px;
    border-color: var(--border-soft);
    height: 36px;
}

Tab {
    color: var(--text-muted);
    background-color: transparent;
    /* transparent border убирает Tab.bottom_line (`build_display_list`
     * рисует rect шириной Tab'а с `border-color`); непрерывную полосу
     * под рядом мы уже получаем от TabBar выше. */
    border-color: transparent;
    border-radius: 0;
    height: 36px;
    font-size: 13px;
    font-weight: 500;
    /* Активный Tab: 2px-индикатор поверх TabBar-border'а, без insets —
     * полоса на всю ширину Tab'а. */
    --tab-indicator-height: 2px;
    --tab-indicator-inset: 0px;
    transition: color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard),
                accent-color var(--duration-fast) var(--ease-standard);
}

Tab:hover {
    color: var(--text);
    background-color: var(--surface-hover);
}

Tab:selected {
    color: var(--text);
    background-color: transparent;
    accent-color: var(--primary);
    font-weight: 600;
}

Tab:selected:hover {
    background-color: var(--surface-hover);
}

Tab:disabled {
    color: var(--text-subtle);
}
