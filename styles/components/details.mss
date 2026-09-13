/* Таб «Детали» правой панели Syn-чата — карточки агент-циклов.
 *
 * Картина: прокручиваемый столбец карточек. Первая — основной цикл чата,
 * дальше живые (и только что завершённые) субагенты в том же столбце, без
 * сдвига по глубине, последняя — размер чата. Каждая карточка складывается из
 * кликабельного заголовка, строки состояния, полосы заполнения KV-ринга и
 * сетки плиток «значение + подпись» 2×N.
 *
 * Все значения — через переменные темы (никаких магических цветов).
 */

/* ── Карточка ─────────────────────────────────────────────────────────────── */

.details-card {
    padding: 12px;
    background-color: var(--bg-panel);
    border-radius: 14px;
    border-width: 1px;
    border-color: var(--border-soft);
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard);
}
.details-card:hover {
    border-color: var(--border);
}

/* Работающий цикл: рамка акцентом — карточку видно, не вчитываясь. */
.details-card-live {
    border-color: var(--primary);
    background-color: var(--surface-selected);
}

/* ── Заголовок карточки ───────────────────────────────────────────────────── */

.details-run-icon {
    icon-size: 18px;
    color: var(--primary);
}
.details-run-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}
.details-run-sub {
    font-size: 11px;
    color: var(--text-muted);
}
.details-run-chevron {
    icon-size: 18px;
    color: var(--text-subtle);
}

/* ── Плашка состояния ─────────────────────────────────────────────────────── */

.details-badge {
    padding: 2px 8px;
    border-radius: var(--radius-pill);
    background-color: var(--bg-search);
    border-width: 1px;
    border-color: var(--border-soft);
}
.details-badge-text {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-muted);
}
.details-badge.live {
    background-color: var(--primary-soft);
    border-color: var(--primary);
}
.details-badge.live .details-badge-text { color: var(--primary); }
.details-badge.ok .details-badge-text   { color: var(--presence-online); }
.details-badge.err {
    border-color: var(--error);
}
.details-badge.err .details-badge-text  { color: var(--error); }
.details-badge.warn {
    border-color: var(--warning);
}
.details-badge.warn .details-badge-text { color: var(--warning); }

/* ── Строка состояния (ход, текущий инструмент, счёт вызовов) ─────────────── */

.details-status {
    padding: 6px 8px;
    border-radius: 8px;
    background-color: var(--bg-search);
}
.details-status-text {
    font-size: 11px;
    color: var(--text-muted);
}

/* ── Полоса заполнения KV-ринга ───────────────────────────────────────────── */

.details-bar {
    padding-top: 2px;
}
.details-bar-label {
    font-size: 11px;
    color: var(--text-muted);
}
.details-bar-value {
    font-size: 11px;
    font-weight: 600;
    color: var(--text);
}
.details-ctx-bar {
    height: 6px;
    border-radius: 3px;
    accent-color: var(--primary);
    background-color: var(--border-soft);
    transition: accent-color var(--duration-med) var(--ease-standard);
}

/* ── Плитки метрик ────────────────────────────────────────────────────────── */

.details-tile {
    padding: 7px 9px;
    border-radius: 10px;
    background-color: var(--bg-search);
    border-width: 1px;
    border-color: transparent;
    transition: border-color var(--duration-fast) var(--ease-standard);
}
.details-tile:hover {
    border-color: var(--border-soft);
}
.details-tile-value {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
}
.details-tile-label {
    font-size: 10px;
    color: var(--text-subtle);
}
/* Скорость декода — главное число карточки, поэтому акцентом. Правило
 * стоит после `.details-tile-value`: у обоих классов одна специфичность,
 * побеждает последнее. */
.details-tile-accent {
    color: var(--primary);
}

/* ── Анимация «живого» индикатора (иконка работающего цикла) ──────────────── */

@keyframes details-pulse {
    0%   { opacity: 0.45; }
    50%  { opacity: 1.0;  }
    100% { opacity: 0.45; }
}
.details-live-dot {
    animation: details-pulse 1.6s var(--ease-standard) infinite;
}
