/* ───────────────────────── Таймеры node-editor'а ────────────────────
 * Два бейджа одной стилистики:
 *   .node-timer    — секундомер ноды, в шапке карточки слева от «×»;
 *   .ne-run-timer  — секундомер всего прогона, в Run-pill за разделителем.
 *
 * Состояния (модификатор на самом бейдже):
 *   --running  зелёный акцент + мягкая пульсация (отсчёт идёт);
 *   --done     приглушённый серый (итог прошлого прогона).
 *
 * DOM:
 *   .node-timer.node-timer--running
 *     ↳ Padding 6/2 → Row gap=4
 *         ↳ .node-timer-icon   (Material «timer»)
 *         ↳ .node-timer-text   («12.4 с» / «1:05.3»)
 *   .ne-run-timer-sep + .ne-run-timer.ne-run-timer--running
 *     ↳ … ↳ .ne-run-timer-icon, .ne-run-timer-text, .ne-run-timer-count
 * ─────────────────────────────────────────────────────────────────── */

/* ───────────────────────── Таймер ноды ─────────────────────────────── */

.node-timer {
    border-radius: var(--radius-pill);
    background-color: rgba(255, 255, 255, 0.06);
    transition: background-color var(--duration-med) var(--ease-standard),
                box-shadow var(--duration-med) var(--ease-standard);
}

.node-timer--running {
    background-color: rgba(34, 197, 94, 0.16);
    box-shadow: inset 0 0 0 1px rgba(34, 197, 94, 0.32);
    animation: ne-timer-breathe 1.8s var(--ease-standard) infinite;
}

.node-timer--done {
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08);
}

.node-timer-icon {
    icon-size: 12px;
    color: #98A0AD;
}

.node-timer-text {
    font-size: 11px;
    font-weight: 600;
    color: #B7BDC8;
    /* Разрядка держит ширину цифр стабильной — бейдж не «дышит»
       по горизонтали на каждой десятой доле. */
    letter-spacing: 0.2px;
}

.node-timer--running .node-timer-icon {
    color: #4ADE80;
}

.node-timer--running .node-timer-text {
    color: #6DD3A8;
}

/* ───────────────────────── Таймер прогона (Run-pill) ───────────────── */

/* Вертикальная черта между кнопками транспорта и таймером. */
.ne-run-timer-sep {
    width: 1px;
    height: 18px;
    background-color: rgba(255, 255, 255, 0.10);
}

.ne-run-timer {
    border-radius: var(--radius-pill);
    background-color: rgba(255, 255, 255, 0.05);
    transition: background-color var(--duration-med) var(--ease-standard),
                box-shadow var(--duration-med) var(--ease-standard);
}

.ne-run-timer--running {
    background-color: rgba(34, 197, 94, 0.16);
    box-shadow: inset 0 0 0 1px rgba(34, 197, 94, 0.30);
    animation: ne-timer-breathe 1.8s var(--ease-standard) infinite;
}

.ne-run-timer-icon {
    icon-size: 13px;
    color: #98A0AD;
}

.ne-run-timer-text {
    font-size: 12px;
    font-weight: 600;
    color: #D8DCE4;
    letter-spacing: 0.2px;
    /* Фиксированная колонка под время: pill центрируется в overlay-строке
       балансировкой краёв, и «прыгающая» ширина сдвигала бы его при
       переходе 9.9 с → 10.0 с. */
    min-width: 46px;
}

.ne-run-timer-count {
    font-size: 10px;
    font-weight: 500;
    color: rgba(216, 220, 228, 0.65);
}

.ne-run-timer--running .ne-run-timer-icon {
    color: #4ADE80;
}

.ne-run-timer--running .ne-run-timer-text {
    color: #6DD3A8;
}

/* ───────────────────────── Пульсация активного отсчёта ────────────── */

/* Тише, чем pulse Run-кнопки: таймер не должен спорить за внимание
   с самой кнопкой, но должен читаться как «идёт прямо сейчас». */
@keyframes ne-timer-breathe {
    0%   { box-shadow: inset 0 0 0 1px rgba(34, 197, 94, 0.32); }
    50%  { box-shadow: inset 0 0 0 1px rgba(34, 197, 94, 0.60); }
    100% { box-shadow: inset 0 0 0 1px rgba(34, 197, 94, 0.32); }
}
