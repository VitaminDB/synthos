/* ─────────────── Панель «Модели в памяти» (правый край canvas'а) ───────
 * Frosted-glass карточка поверх нодового полотна: показывает, что сейчас
 * реально держит VRAM/RAM, и даёт выгрузить это по одной модели.
 * Цвета — тематические `--glass-*` токены (их задаёт SynthosTheme::to_mss
 * под каждую тему), поэтому панель одинаково читается и на светлом, и на
 * тёмном полотне. Захардкоженного белого здесь нет намеренно.
 *
 * DOM:
 *   .ne-models-panel[.ne-models-panel--collapsed]
 *     ↳ .ne-models-head → .ne-models-head-icon .ne-models-title
 *                         .ne-models-count .ne-models-chevron
 *     ↳ .ne-models-vram
 *     ↳ .ne-models-list → .ne-models-row × N
 *                          ↳ .ne-models-row-title
 *                          ↳ .ne-models-row-label
 *                          ↳ .ne-models-row-meta
 *                          ↳ .ne-models-unload
 * ─────────────────────────────────────────────────────────────────────── */

.ne-models-panel {
    width: 268px;
    background-color: var(--glass-card-bg);
    backdrop-filter: blur(24px);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--glass-border);
    box-shadow: var(--glass-shadow);
    transition: box-shadow var(--duration-med) var(--ease-standard),
                border-color var(--duration-med) var(--ease-standard);
}

/* Свёрнутая панель — только шапка. Ширину не жмём: прыгающий по ширине
 * оверлей читается как глитч, а не как сворачивание. */
.ne-models-panel--collapsed {
    box-shadow: 0 8px 22px rgba(0, 0, 0, 0.18);
}

/* ───────────────────────── Шапка ──────────────────────────────────── */

.ne-models-head {
    background-color: transparent;
    border-radius: var(--radius-panel);
}

.ne-models-head-icon {
    color: var(--text-muted);
    icon-size: 16px;
}

.ne-models-title {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
}

/* Счётчик-пилюля справа от заголовка. Полупрозрачная подложка вместо
 * `--surface-hover`: на стекле непрозрачная плашка выглядит наклейкой. */
.ne-models-count {
    color: var(--text-muted);
    background-color: rgba(127, 133, 148, 0.16);
    border-radius: var(--radius-pill);
    padding: 1px 7px 1px 7px;
    font-size: 11px;
}

.ne-models-chevron {
    color: var(--text-muted);
    background-color: transparent;
    border-radius: var(--radius-pill);
    padding: 4px 4px 4px 4px;
    icon-size: 16px;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.ne-models-chevron:hover {
    background-color: rgba(127, 133, 148, 0.18);
    color: var(--text);
}

/* ───────────────────────── VRAM-строка ────────────────────────────── */

.ne-models-vram {
    color: var(--text-subtle);
    font-size: 11px;
}

/* Заглушка на машинах без GPU: место под строку не резервируем. */
.ne-models-vram-empty {
    background-color: transparent;
    height: 0px;
}

.ne-models-empty {
    color: var(--text-subtle);
    font-size: 12px;
}

/* ───────────────────────── Список ─────────────────────────────────── */

/* Потолок высоты: при десятке загруженных компонентов панель не должна
 * закрывать полотно — дальше скроллом. */
.ne-models-list {
    max-height: 360px;
}

.ne-models-row {
    background-color: var(--glass-field-bg);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--glass-border);
    transition: border-color var(--duration-fast) var(--ease-standard);
}

.ne-models-row:hover {
    border-color: var(--primary);
}

.ne-models-row-title {
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
}

/* Имя файла бандла — самая длинная строка; обрезаем многоточием, полное
 * значение всё равно видно в логе `models`. */
.ne-models-row-label {
    color: var(--text-muted);
    font-size: 11px;
    max-width: 180px;
    text-overflow: ellipsis;
}

.ne-models-row-meta {
    color: var(--text-subtle);
    font-size: 11px;
}

.ne-models-unload {
    color: var(--text-muted);
    background-color: transparent;
    border-radius: var(--radius-pill);
    padding: 5px 5px 5px 5px;
    icon-size: 18px;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.ne-models-unload:hover {
    background-color: rgba(229, 83, 83, 0.16);
    color: var(--error);
}
