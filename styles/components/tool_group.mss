/* Сворачиваемая группа подряд идущих tool-вызовов одного типа.
 * Применяется только в minimal-режиме отображения tool-карточек.
 * Все цвета через MSS-переменные, никаких хардкодов. */

/* ───────────────────────── Group card ───────────────────────── */

.tool-group-card {
    background-color: var(--bg-panel);
    border-radius: 14px;
    border-width: 1px;
    border-color: var(--border);
    padding: 10px 12px;
    /* Тот же max-width, что у одиночных tool-карточек, чтобы при
     * переключении режимов лента не «прыгала» по ширине. */
    max-width: 90%;
    transition: border-color var(--duration-med) var(--ease-standard),
                background-color var(--duration-med) var(--ease-standard);
    animation: bubble-pop-in var(--duration-med) var(--ease-standard);
}

.tool-group-card:hover {
    border-color: var(--primary);
    background-color: var(--surface-hover);
}

/* Цепочка, которая упала целиком: ни одного успешного вызова. Тот же тон,
 * что у `.tool-result-card-error` в `tool_messages.mss` — пользователь сразу
 * видит, что внутри всё плохо, не разворачивая. */
.tool-group-card-with-error {
    border-color: var(--error);
    background-color: rgba(229, 83, 83, 0.05);
}

/* Смешанный исход (часть вызовов прошла, часть упала): заливку не трогаем —
 * одна ошибка из семи не делает цепочку сломанной. Хватает приглушённой
 * рамки, подробности — в счётчиках шапки. */
.tool-group-card-mixed {
    border-color: rgba(229, 83, 83, 0.35);
}

/* ───────────────────────── Header ───────────────────────── */

/* Мягкая плашка под иконкой инструмента — тот же приём, что у
 * `.tool-result-avatar`: шапка цепочки читается как заголовок карточки. */
.tool-group-icon-wrap {
    width: 28px;
    height: 28px;
    border-radius: 9px;
    background-color: var(--primary-soft);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.tool-group-icon {
    color: var(--primary);
    font-size: 16px;
}

.tool-group-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}

/* «Пилюли» с исходом цепочки: сколько вызовов прошло (галочка) и сколько
 * упало. Цвета — те же, что у статус-иконок одиночных result-карточек в
 * `tool_messages.mss`, чтобы свёрнутая группа читалась как их сумма. */
.tool-group-count {
    border-radius: 999px;
    padding: 2px 8px;
}

.tool-group-count-icon {
    font-size: 14px;
}

.tool-group-count-text {
    font-size: 12px;
    font-weight: 600;
}

.tool-group-count-ok {
    background-color: rgba(34, 197, 94, 0.12);
}

.tool-group-count-ok .tool-group-count-icon,
.tool-group-count-ok .tool-group-count-text {
    color: var(--presence-online);
}

.tool-group-count-error {
    background-color: rgba(229, 83, 83, 0.12);
}

.tool-group-count-error .tool-group-count-icon,
.tool-group-count-error .tool-group-count-text {
    color: var(--error);
}

.tool-group-chevron {
    color: var(--text-muted);
    font-size: 18px;
    transition: color var(--duration-fast) var(--ease-standard);
}

.tool-group-card:hover .tool-group-chevron {
    color: var(--primary);
}

/* ───────────────────────── Body (раскрытое содержимое) ───────────────────────── */

/* Раскрытая цепочка — лента шагов вдоль вертикальной направляющей: она
 * связывает вызовы в одну работу, вместо стопки самостоятельных карточек. */
.tool-group-children {
    padding-top: 6px;
    padding-left: 12px;
    margin-left: 2px;
    border-left-width: 2px;
    border-left-color: var(--border);
}

/* Внутри цепочки вложенные карточки теряют собственную рамку и фон: 14
 * бордюров подряд (7 вызовов × call+result) — это шум, а не структура.
 * Границы группы держит сама карточка группы. */
.tool-group-children .tool-call-card,
.tool-group-children .tool-result-card {
    /* `:hover` внутри потомкового селектора движок MSS применяет как
     * обычное правило (состояние теряется), поэтому подсветки шага здесь
     * нет — hover-состояние остаётся за самими `.tool-call-card:hover`. */
    background-color: transparent;
    border-width: 0;
    border-radius: 10px;
    padding: 6px 8px;
    /* 90% от ширины группы дало бы рваный правый край внутри и без того
     * ограниченной карточки. */
    max-width: 100%;
}

/* Упавший шаг остаётся видимым и без рамки — по заливке. */
.tool-group-children .tool-result-card-error {
    background-color: rgba(229, 83, 83, 0.07);
}
