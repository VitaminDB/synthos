/* Left-hand icon rail. */

.nav-rail {
    background-color: var(--bg-rail);
    border-right-width: 1px;
    border-color: var(--border-soft);
    padding: 16px 12px 16px 12px;
    width: 72px;
    height: 100%;
}

/* Брендовый логотип synthos в верхушке рельса. Контейнер 40×40
 * соответствует диаметру кнопок (.nav-rail-item) и аватара в footer'е —
 * визуальный ритм рельса остаётся выдержанным. Фон-цвет не задаём:
 * SVG-иконка сама несёт squircle-фон (см. packaging/synthos.svg).
 * border-radius у контейнера дублирует squircle на случай, если
 * растеризованная rgba-маска имеет subpixel-overflow по краям. */
.nav-rail-logo {
    width: 40px;
    height: 40px;
    border-radius: 12px;
}

.nav-rail-item {
    /* Размер кнопки и иконки — оба через MSS (ToolButton-builder size()
     * удалён, см. syngui/widgets/buttons/tool_button.rs). Hit-area 40×40
     * даёт заметный hover/selected фон вокруг 24-px глифа. */
    width: 40px;
    height: 40px;
    icon-size: 24px;
    border-radius: 12px;
    background-color: transparent;
    color: var(--text-muted);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.nav-rail-item:hover {
    background-color: var(--surface-hover);
}

.nav-rail-item.selected {
    background-color: var(--primary-soft);
    color: var(--primary);
}

.nav-rail-divider {
    width: 32px;
    height: 1px;
    background-color: var(--border-soft);
}

/* ─── Динамические code-сессии ─────────────────────────────────────────
 * Сегмент между primary-кластером и кнопкой `+`. Содержит реактивный
 * список сессий (по одной кнопке-папке на каждую) и сам `+`.
 * Стили per-item наследуются от `.nav-rail-item`; здесь — только
 * акцент для кнопки добавления, чтобы визуально отделить её от сессий
 * без потери симметрии.
 * ──────────────────────────────────────────────────────────────────── */

.nav-rail-sessions {
    /* Без фиксированной высоты — сегмент растёт под количеством сессий.
     * При большом количестве sidebar остаётся scrollable благодаря
     * SpaceBetween в основном Column'е (top уезжает вверх, footer
     * фиксирован). */
}

.nav-rail-item-add {
    color: var(--text-muted);
    transition: color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}

.nav-rail-item-add:hover {
    background-color: var(--surface-hover);
    color: var(--primary);
}

/* Бейдж количества открытых терминалов на плитке code-сессии.
 * Цвет — сводка занятости терминалов (busy = выполняется команда ИЛИ
 * вывод обновлялся за последнюю секунду, см. terminal_activity):
 *   busy  — все заняты        → зелёный;
 *   idle  — все простаивают   → красный;
 *   mixed — часть занята      → оранжевый.
 * padding ужимает кружок: высота Badge = font(10px) + 2·padding,
 * дефолтные 8px давали ~26px — непропорционально много на 40px-кнопке. */
.nav-rail-term-badge {
    padding: 3px;
}

.nav-rail-term-badge.busy {
    background-color: var(--presence-online);
}

.nav-rail-term-badge.idle {
    background-color: var(--error);
}

.nav-rail-term-badge.mixed {
    background-color: var(--warning);
}

/* Подпись под иконкой проекта. line-clamp:1 + ellipsis даёт «autosize»
 * (растёт по содержимому до доступной ширины Column'а) и «clamp len»
 * (длинные имена обрезаются многоточием в одну строку).
 * Полное имя остаётся доступно через tooltip ToolButton'а. */
.nav-rail-session-label {
    color: var(--text-muted);
    font-size: 9px;
    font-weight: 500;
    line-clamp: 1;
    max-width: 48px;
    text-align: center;
    transition: color var(--duration-fast) var(--ease-standard);
}

.nav-rail-session-label.selected {
    color: var(--primary);
}
