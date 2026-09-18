/* Нижняя панель загрузок страницы HuggingFace: сводка на всю ширину окна,
 * под ней (если развёрнута) — очередь и настройки. */

.hf-dock {
    background-color: var(--bg-shell);
    border-top-width: 1px;
    border-top-color: var(--border-soft);
}

.hf-dock-summary {
    padding: 8px var(--spacing-md);
}

.hf-dock-title-icon {
    icon-size: 18px;
    color: var(--primary);
}

.hf-dock-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}

/* Общий прогресс. Rail тот же, что у строки файла, но тоньше. */
.hf-dock-rail {
    height: 6px;
    border-radius: 3px;
    background-color: rgba(148, 163, 184, 0.22);
    overflow: hidden;
}

.hf-dock-rail-fill {
    height: 6px;
    border-radius: 3px;
    background-color: var(--primary);
    transition: width var(--duration-fast) var(--ease-standard);
}

/* Пауза или нечему качаться (остались только ошибки/остановленные). */
.hf-dock-rail--idle .hf-dock-rail-fill {
    background-color: var(--warning);
}

.hf-dock-percent {
    font-size: 12px;
    font-weight: 600;
    font-family: monospace;
    color: var(--text);
}

.hf-dock-stats {
    font-size: 11px;
    color: var(--text-muted);
    line-clamp: 1;
}

/* Сводка настроек и «Изменить…» — нейтральная пилюля. */
.hf-dock-chip {
    padding: 6px 12px;
    background-color: var(--surface-hover);
    color: var(--text-muted);
    border-radius: var(--radius-pill);
    font-size: 12px;
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.hf-dock-chip:hover {
    background-color: var(--surface-selected);
    color: var(--text);
}

.hf-dock-pause-all {
    padding: 6px 14px;
    background-color: var(--surface-hover);
    color: var(--text);
    border-radius: var(--radius-pill);
    font-size: 12px;
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.hf-dock-pause-all:hover {
    background-color: var(--surface-selected);
}

/* На паузе — акцент: есть что возобновить. */
.hf-dock-pause-all.paused {
    background-color: var(--primary);
    color: var(--on-primary);
}

.hf-dock-body-empty {
    height: 0;
}

.hf-dock-body {
    height: 260px;
    border-top-width: 1px;
    border-top-color: var(--border-soft);
}

.hf-dock-queue {
    padding: var(--spacing-sm) var(--spacing-md);
    height: 100%;
}

.hf-dock-settings {
    width: 400px;
    height: 100%;
    padding: var(--spacing-sm) var(--spacing-md);
    border-left-width: 1px;
    border-left-color: var(--border-soft);
}

.hf-dock-scroll {
    height: 100%;
}

.hf-dock-empty,
.hf-dock-more {
    font-size: 12px;
    color: var(--text-subtle);
}

.hf-dock-more {
    padding: 6px 8px;
}

.hf-dock-row {
    padding: 6px 8px;
    border-radius: 10px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.hf-dock-row:hover {
    background-color: var(--surface-hover);
}

.hf-dock-row-name {
    font-size: 12px;
    font-weight: 500;
    color: var(--text);
    line-clamp: 1;
}

/* Репозиторий — ссылка на его файлы. */
.hf-dock-row-repo {
    padding: 1px 8px;
    border-radius: var(--radius-pill);
    background-color: transparent;
    color: var(--text-subtle);
    font-size: 11px;
}

.hf-dock-row-repo:hover {
    background-color: var(--primary-soft);
    color: var(--primary);
}

.hf-dock-row-size {
    font-size: 11px;
    color: var(--text-muted);
    font-family: monospace;
}

.hf-dock-settings-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
}

.hf-dock-setting-label {
    font-size: 13px;
    color: var(--text);
}

.hf-dock-setting-hint {
    font-size: 11px;
    color: var(--text-subtle);
}

.hf-dock-spin {
    width: 96px;
}
