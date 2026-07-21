/* Details tab (правый сайдбар) — дашборд метрик llama + системы.
 *
 * Картина: прокручиваемый столбец из carded-секций. Каждая карточка
 * содержит иконку + заголовок + тело. Тело варьируется:
 *  - большие числа (producer per-second),
 *  - мини-графики LineChart (history),
 *  - ProgressBar-полоски per-core.
 *
 * Все значения — через переменные темы (никаких магических цветов).
 */

.details-title-box {
    /* Обёртка для заголовка — Text не имеет собственного padding, поэтому
     * отступ задаём на DecoratedBox.class("details-title-box"). */
    padding-left: 4px;
    padding-right: 4px;
    padding-bottom: 4px;
}
.details-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
}

.details-card {
    padding: 14px;
    background-color: var(--bg-panel);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard);
}
.details-card:hover {
    background-color: var(--surface-hover);
    border-color: var(--border);
}

.details-card-icon {
    icon-size: 18px;
    color: var(--primary);
}
.details-card-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
    letter-spacing: 0.5px;
}

/* ── Большие числовые метрики (prompt / predicted tokens/sec) ──────────────── */
.details-big-metric {
    padding-right: 16px;
}
.details-metric-big {
    font-size: 22px;
    font-weight: 700;
    color: var(--text);
}
.details-metric-unit {
    font-size: 11px;
    color: var(--text-muted);
    padding-top: 2px;
}

/* ── Лейбл/значение в строковых метриках ──────────────────────────────────── */
.details-metric-label {
    font-size: 12px;
    color: var(--text-muted);
}
.details-metric-value {
    font-size: 13px;
    color: var(--text);
    font-weight: 500;
}
.details-metric-icon {
    icon-size: 14px;
    color: var(--text-muted);
}

/* ── Подзаголовок (например «Ядра CPU») ────────────────────────────────────── */
.details-subheader {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    letter-spacing: 0.4px;
    padding-top: 4px;
}

/* ── Мини-графики ──────────────────────────────────────────────────────────── */
.details-mini-chart {
    height: 120px;
    margin-top: 4px;
    color: var(--text-muted);
    grid-color: var(--border-soft);
    axis-color: var(--border-soft);
    axis-font-size: 8px;
    point-size: 2px;
}
.details-mini-chart-empty {
    height: 120px;
    background-color: var(--bg-search);
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border-soft);
}

/* График GPU/VRAM внутри объединённой карточки «Система» — чуть выше
 * mini-chart, чтобы линии и значения читались. Маркеры точек ужаты,
 * чтобы плотный history-ряд не превращался в гирлянду. */
.details-system-chart {
    height: 110px;
    margin-top: 4px;
    color: var(--text-muted);
    grid-color: var(--border-soft);
    axis-color: var(--border-soft);
    axis-font-size: 8px;
    point-size: 2px;
}

/* ── Гейджи (CPU/RAM/GPU/VRAM dial) ────────────────────────────────────────── */
.details-dial {
    padding: 4px 0px;
}
.details-dial-ring {
    /* CircularProgress читает width/height из MSS как диаметр. */
    width: 56px;
    height: 56px;
    color: var(--primary);
    transition: color var(--duration-med) var(--ease-standard);
}
.details-dial-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    letter-spacing: 0.3px;
    padding-top: 4px;
}
.details-dial-caption {
    font-size: 11px;
    color: var(--text);
    font-weight: 500;
}

/* ── Прогресс-бар контекста n_ctx ──────────────────────────────────────────── */
.details-ctx-bar {
    height: 8px;
    border-radius: 4px;
    accent-color: var(--primary);
    background-color: var(--border-soft);
    transition: accent-color var(--duration-med) var(--ease-standard);
}

/* ── BarChart «Ядра CPU» ───────────────────────────────────────────────────── */
.details-cpu-cores {
    /* Плотный компактный ряд столбиков: мелкий шрифт подписей и %-делений,
     * без лишних осей. BarChart берёт axis-font-size/grid-color как MSS-свойства. */
    height: 120px;
    axis-font-size: 8px;
    color: var(--text-muted);
    grid-color: var(--border-soft);
}

/* ── GPU: плашка «недоступно» ──────────────────────────────────────────────── */
.details-gpu-unavailable {
    padding: 10px 12px;
    background-color: var(--bg-search);
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border-soft);
}
.details-gpu-warn-icon {
    icon-size: 18px;
    color: var(--primary);
}

/* ── Анимация «живого» индикатора (когда pending=true — используется где нужно) ── */
@keyframes details-pulse {
    0%   { opacity: 0.55; }
    50%  { opacity: 1.0;  }
    100% { opacity: 0.55; }
}
.details-live-dot {
    animation: details-pulse 1.6s var(--ease-standard) infinite;
}
