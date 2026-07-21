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

/* Группа, в которой хотя бы один результат — ошибка. Тот же тон, что у
 * `.tool-result-card-error` в `tool_messages.mss` — пользователь сразу
 * видит, что внутри есть проблема, не разворачивая. */
.tool-group-card-with-error {
    border-color: var(--error);
    background-color: rgba(229, 83, 83, 0.05);
}

/* ───────────────────────── Header ───────────────────────── */

.tool-group-icon {
    color: var(--primary);
    font-size: 18px;
}

.tool-group-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}

/* «Пилюля» со счётчиком вызовов — visual-balance с иконкой инструмента,
 * чтобы при беглом взгляде сразу считать N. */
.tool-group-count {
    font-size: 12px;
    font-weight: 600;
    color: var(--primary);
    background-color: var(--primary-soft);
    border-radius: 999px;
    padding: 2px 8px;
}

.tool-group-status-error {
    color: var(--error);
    font-size: 16px;
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

.tool-group-children {
    /* Просто контейнер для выравнивания; визуальные акценты на самих
     * вложенных карточках (.tool-call-card / .tool-result-card). */
    padding-top: 4px;
}
