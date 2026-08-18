/* ───────────────────────── Терминал-табы (Zed/VSCode-style) ──────────
 * Стилизация верхнего бара с табами в нижней панели code editor'а.
 *
 * Структура DOM (см. terminal_pane.rs::tabs_bar/tab_chip):
 *
 *   .term-tabs-bar  ── фон бара, нижний 1px-border
 *     ↳ Row
 *         ↳ GestureDetector  (on_click → set_active_terminal)
 *             ↳ .term-tab[--active]   ── одна вкладка
 *                 ↳ .term-tab-icon    ── MI_TERMINAL
 *                 ↳ .term-tab-title   ── заголовок (OSC 0/2 reactive)
 *                     ↳ .term-tab-title-text
 *                 ↳ .term-tab-close   ── × кнопка (ToolButton)
 *         ↳ ... ещё табы
 *         ↳ .term-tab-add             ── + ToolButton
 *
 * Эстетика — Zed editor: тонкий бар (~32px), активный таб подсвечен
 * accent-полосой снизу + чуть светлее фона. Плавные transitions на
 * background и opacity. Material icons через `font-family`.
 * ─────────────────────────────────────────────────────────────────── */

/* ───────────────────────── Сам бар ─────────────────────────────────── */

.term-tabs-bar {
    background-color: var(--bg-panel);
    border-bottom-width: 1px;
    border-color: var(--border);
    height: 34px;
    padding: 0px 4px 0px 0px;
}

/*
 * Spacer между add-кнопкой и gear-кнопкой. flex-grow растягивает
 * пустую полосу до правого края tab-bar'а — gear «прижимается» к нему.
 */
.term-tabs-spacer {
    background-color: transparent;
    flex-grow: 1;
}

/*
 * Active-area — содержит виджет Terminal либо empty placeholder.
 * flex-grow=1 даёт ей весь остаток вертикального пространства Column'а
 * после фиксированного 34px tab-bar'а. Без этого Terminal получает
 * intrinsic height (≈40px) и до первого drag сплиттера не растягивается.
 */
.term-active-area {
    background-color: transparent;
    flex-grow: 1;
}

/* ───────────────────────── Один таб ────────────────────────────────── */

/*
 * Padding 0/12/0/10 — асимметрия: иконка слева ближе к границе (10px),
 * close-кнопка справа имеет внутренний padding и компенсирует визуальную
 * симметрию (12px). Точно как в Zed.
 *
 * border-right — vertical-divider между табами (1px светлее border'а).
 * На активном — этот divider убирается через --active.
 */
.term-tab {
    /* Высота фиксирована (= высоте `.term-tabs-bar` минус 1px-border-bottom);
     * раньше высоту неявно тянул ToolButton.size(N), теперь size-builder
     * удалён — задаём через MSS, чтобы вкладка не схлопывалась до intrinsic
     * высоты текста (~20 px) при коротких title. */
    height: 33px;
    /* shrink-to-fit по контенту: title содержит свой `max-width: 300px`, который
     * становится фактическим лимитом ширины tab'а (вместе с иконками + paddings).
     * Без `width: fit-content` Row родителя растягивал вкладку на всю ширину
     * bar'а и close-кнопка уезжала за пределы. */
    width: fit-content;
    background-color: transparent;
    padding: 0px 12px 0px 10px;
    border-right-width: 1px;
    border-color: var(--border);
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);
}
.term-tab:hover {
    background-color: var(--surface-hover);
}

/*
 * Активный таб: `--bg-search` (заметно светлее `--bg-panel` бара) +
 * accent-полоса снизу (2px). Точная высота
 * полосы — через border-bottom-width: 2px поверх 1px-border'а бара.
 *
 * `border-bottom-color: var(--primary)` (per-side) аккуратно перекрашивает
 * только нижнюю кромку. Общий `border-color: var(--border)` оставлен на
 * правом 1px-divider'е между табами — иначе он бы стал accent-синим, что
 * визуально сливало активный таб с соседом.
 */
.term-tab--active {
    background-color: var(--bg-search);
    border-color: var(--border);
    border-bottom-width: 2px;
    border-bottom-color: var(--primary);
}
.term-tab--active:hover {
    background-color: var(--bg-search);
}

/* ───────────────────────── Контент таба ─────────────────────────────── */

.term-tab-icon {
    font-family: "Material Icons";
    font-size: 16px;
    color: var(--text-muted);
    transition: color var(--duration-fast) var(--ease-standard);
}
.term-tab--active .term-tab-icon {
    color: var(--text);
}

.term-tab-title {
    background-color: transparent;
    /*
     * Title shrink-to-fit по тексту до max-width: 300px. Без min-width —
     * иначе fit-content родителя стрижёт по min-content title и обрезает
     * короткие заголовки до min-width-символов.
     */
    max-width: 300px;
}
.term-tab-title-text {
    color: var(--text-muted);
    font-size: 13px;
    /*
     * line-clamp:1 + max-width на родителе .term-tab-title (200px) делает
     * длинные заголовки усечёнными с ellipsis вместо wrap'а на вторую строку.
     * См. memory entry «Text max_lines / MSS line-clamp 2026-04-25».
     */
    line-clamp: 1;
    transition: color var(--duration-fast) var(--ease-standard);
}
.term-tab--active .term-tab-title-text {
    color: var(--text);
    font-weight: 500;
}
.term-tab:hover .term-tab-title-text {
    color: var(--text);
}

/* ───────────────────────── Close-кнопка таба ───────────────────────── */

.term-tab-close {
    color: var(--text-muted);
    background-color: transparent;
    border-radius: var(--radius-pill);
    padding: 2px 2px 2px 2px;
    icon-size: 14px;
    opacity: 0.0;
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}
/*
 * Close-кнопка появляется только на hover/active таба — иначе слишком
 * много визуального шума при множестве вкладок. Стандарт VSCode/Zed.
 */
.term-tab:hover .term-tab-close {
    opacity: 0.6;
}
.term-tab--active .term-tab-close {
    opacity: 0.6;
}
.term-tab-close:hover {
    opacity: 1.0;
    color: var(--text);
    background-color: var(--surface-hover);
}

/* ───────────────────────── Add-кнопка (+) ─────────────────────────── */

.term-tab-add {
    color: var(--text-muted);
    background-color: transparent;
    border-radius: var(--radius-pill);
    padding: 6px 6px 6px 6px;
    icon-size: 16px;
    opacity: 0.7;
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}
.term-tab-add:hover {
    opacity: 1.0;
    color: var(--text);
    background-color: var(--surface-hover);
}

/* ───────────────────────── Gear-кнопка в tab-bar ───────────────────
 * Override для `.term-gear-btn` из code_editor_terminal.mss: исходные
 * стили (тёмный полупрозрачный фон, белый hover-color) рассчитаны на
 * overlay поверх тёмного терминала. В tab-bar фон светлый
 * (`var(--bg-panel)`), белая hover-иконка визуально «пропадает».
 * Перебиваем на тот же стиль, что и `.term-tab-add` — единый внешний
 * вид для всех action-кнопок tab-bar'а.
 * ─────────────────────────────────────────────────────────────────── */
.term-gear-btn {
    color: var(--text-muted);
    background-color: transparent;
    border-radius: var(--radius-pill);
    padding: 6px 6px 6px 6px;
    icon-size: 16px;
    opacity: 0.7;
    cursor: pointer;
    transition: opacity var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}
.term-gear-btn:hover {
    opacity: 1.0;
    color: var(--text);
    background-color: var(--surface-hover);
}

/* ───────────────────────── Empty placeholder ───────────────────────── */

/*
 * Показывается когда нет ни одной вкладки. Subtle — не должен
 * привлекать слишком много внимания, это «состояние покоя».
 */
.term-empty-icon {
    font-family: "Material Icons";
    font-size: 32px;
    color: var(--text-muted);
    opacity: 0.5;
}
.term-empty-title {
    color: var(--text-muted);
    font-size: 14px;
    font-weight: 500;
}
.term-empty-hint {
    color: var(--text-muted);
    font-size: 12px;
    opacity: 0.7;
}
