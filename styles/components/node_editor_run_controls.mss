/* ───────────────────────── Run / Pause / Stop pill ─────────────────
 * Floating pill в верхнем-правом углу canvas'а. Активная кнопка
 * подсвечена и пульсирует (keyframe). Состояние pill'а тоже опционально
 * подсвечивает фон (.ne-run-controls--running и т.п.).
 *
 * DOM:
 *   .ne-run-controls.ne-run-controls--{stopped|running|paused}
 *     ↳ Padding 8/4
 *         ↳ Row gap=2
 *             ↳ .ne-run-btn.ne-run-btn--play[--active]
 *             ↳ .ne-run-btn.ne-run-btn--pause[--active]
 *             ↳ .ne-run-btn.ne-run-btn--stop[--active]
 * ─────────────────────────────────────────────────────────────────── */

.ne-run-controls {
    background-color: rgba(35, 37, 46, 0.92);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-pill);
    box-shadow: 0 8px 22px rgba(0, 0, 0, 0.35);
    transition: box-shadow var(--duration-med) var(--ease-standard),
                background-color var(--duration-med) var(--ease-standard),
                border-color var(--duration-med) var(--ease-standard);
}

.ne-run-controls--running {
    border-color: rgba(34, 197, 94, 0.45);
    box-shadow: 0 8px 22px rgba(34, 197, 94, 0.20);
}

.ne-run-controls--paused {
    border-color: rgba(234, 179, 8, 0.45);
    box-shadow: 0 8px 22px rgba(234, 179, 8, 0.18);
}

/* ───────────────────────── Кнопки ───────────────────────────────────── */

.ne-run-btn {
    color: #D8DCE4;
    background-color: transparent;
    border-radius: var(--radius-pill);
    padding: 6px 10px 6px 10px;
    icon-size: 16px;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard),
                box-shadow var(--duration-med) var(--ease-standard);
}

.ne-run-btn:hover {
    background-color: rgba(255, 255, 255, 0.08);
    color: #FFFFFF;
}

/* Per-button accent-цвета на active. Чтобы pulse-keyframes тоже
 * читались — anim animates box-shadow через переменную на самой кнопке. */
.ne-run-btn--play.ne-run-btn--active {
    background-color: rgba(34, 197, 94, 0.20);
    color: #4ADE80;
    box-shadow: 0 0 0 0 rgba(34, 197, 94, 0.55);
    animation: ne-run-pulse-green 1.6s var(--ease-standard) infinite;
}
.ne-run-btn--pause.ne-run-btn--active {
    background-color: rgba(234, 179, 8, 0.20);
    color: #FACC15;
}
.ne-run-btn--stop.ne-run-btn--active {
    background-color: rgba(239, 68, 68, 0.20);
    color: #F87171;
}

/* ───────────────────────── Pulse keyframe (Run-only) ───────────────── */

@keyframes ne-run-pulse-green {
    0%   { box-shadow: 0 0 0 0   rgba(34, 197, 94, 0.55); }
    70%  { box-shadow: 0 0 0 10px rgba(34, 197, 94, 0.00); }
    100% { box-shadow: 0 0 0 0   rgba(34, 197, 94, 0.00); }
}

/* ───────────────────────── Overlay-row (toolbar+pill) ─────────────── */

/* Невидимый balance-box справа в overlay-Row: визуально пустой, но
 * имеет ту же ширину, что toolbar слева. SpaceBetween-распределение
 * расталкивает три child'а с равными gap'ами; одинаковые края → run-pill
 * по центру. Высота = pill, чтобы Stretch не задирал её до tall row'а. */
.ne-overlay-balance {
    background-color: transparent;
    width: 160px;
    height: 36px;
}
