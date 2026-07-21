/* KB chip в left-toolbar input-панели + popover (Portal-overlay). */

/* ── Chip-кнопка ───────────────────────────────────────────────────────── */

/* Обёртка GestureDetector — собственных стилей нет; служит «hook»'ом для
 * future-fix'а (например, focus-ring). Сам chip — DecoratedBox внутри. */
.input-kb-chip-wrap {
    background-color: transparent;
}

.input-kb-chip {
    background-color: var(--surface-hover);
    border-width: 1px;
    border-color: var(--border);
    border-radius: var(--radius-pill);
    padding: 6px 12px 6px 10px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color     var(--duration-fast) var(--ease-standard),
                color            var(--duration-fast) var(--ease-standard);
}

.input-kb-chip:hover {
    background-color: var(--surface-selected);
    border-color: var(--primary);
}

.input-kb-chip-icon {
    icon-size: 18px;
    color: var(--primary);
}

.input-kb-chip-text {
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
}

/* «Пусто» (ни одной активной коллекции): приглушённый стиль — пользователь
 * видит, что chip присутствует, но коллекции не подключены. */
.input-kb-chip-empty {
    background-color: transparent;
    border-color: var(--border-soft);
}

.input-kb-chip-empty .input-kb-chip-icon {
    color: var(--text-muted);
}

.input-kb-chip-empty .input-kb-chip-text {
    color: var(--text-muted);
    font-weight: 400;
}

/* ── Индикатор «идёт augment-поиск» ──────────────────────────────────── */

.input-kb-progress {
    background-color: var(--primary-soft);
    border-radius: var(--radius-pill);
    padding: 6px 12px 6px 10px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.input-kb-progress-icon {
    icon-size: 16px;
    color: var(--primary);
    animation: input-kb-progress-pulse 1.2s ease-in-out infinite;
}

.input-kb-progress-text {
    color: var(--primary);
    font-size: 12px;
    font-weight: 500;
}

/* Idle: zero-size, ни padding'а, ни цвета — Row не оставит «дыры». */
.input-kb-progress-empty {
    width: 0;
    height: 0;
    background-color: transparent;
    border-width: 0;
    padding: 0 0 0 0;
}

@keyframes input-kb-progress-pulse {
    0%   { opacity: 1.0; }
    50%  { opacity: 0.45; }
    100% { opacity: 1.0; }
}

/* ── Popover-карточка ─────────────────────────────────────────────────── */
/* TODO: PortalAnchor сейчас viewport-based (BottomStart с margin'ами в
 * kb_chip.rs). Если положение chip'а в toolbar'е сильно сместится после
 * редизайна — отрегулировать margin_bottom / margin_left в kb_chip.rs
 * или добавить новый PortalAnchor::AbsolutePoint во фреймворке syngui. */

.kb-chip-card {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    border-radius: var(--radius-panel);
    padding: 14px 14px 12px 14px;
    box-shadow: var(--glass-shadow);
    /* Высота — auto по содержимому. Длинные списки коллекций ограничены
     * ScrollView внутри `.kb-chip-list-wrap`; max-height на самой карточке
     * обрезал бы footer и кнопку «Открыть настройки» за нижний край. */
}

.kb-chip-card-empty {
    padding: 18px 16px 14px 16px;
}

.kb-chip-header-icon {
    icon-size: 22px;
    color: var(--primary);
}

.kb-chip-title {
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
}

.kb-chip-close {
    color: var(--text-muted);
    border-radius: 6px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color            var(--duration-fast) var(--ease-standard);
}

.kb-chip-close:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.kb-chip-divider {
    height: 1px;
    background-color: var(--border-soft);
}

.kb-chip-switch-row {
    padding: 2px 2px 2px 2px;
}

/* Checkbox без явных MSS-цветов рисуется белым квадратом с серой
 * рамкой (см. widgets/input/checkbox.rs:169-172 — захардкоженные fallback'и
 * Color::WHITE/#D1D5DB/#374151). Привязываем к переменным темы. */
Checkbox.kb-chip-switch,
Checkbox.kb-chip-row-cbox {
    color: var(--text);
    background-color: var(--surface-hover);
    border-color: var(--border-strong);
    accent-color: var(--primary);
    font-size: 13px;
}

/* ── Список коллекций ─────────────────────────────────────────────────── */

/* Скроллируемый контейнер для длинного списка. Ограничиваем высоту,
 * чтобы popover не вырастал на весь экран при 30 коллекциях. */
.kb-chip-list-wrap {
    background-color: transparent;
    max-height: 260px;
}

.kb-chip-list-scroll {
    background-color: transparent;
}

.kb-chip-row {
    background-color: transparent;
    border-radius: 8px;
    padding: 8px 8px 8px 8px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.kb-chip-row:hover {
    background-color: var(--surface-hover);
}

.kb-chip-row-active {
    background-color: var(--primary-soft);
}

.kb-chip-row-name {
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
}

.kb-chip-row-meta {
    color: var(--text-muted);
    font-size: 11px;
}

.kb-chip-row-cbox {
    /* Checkbox — toggle отрабатывает родительский GestureDetector,
     * поэтому здесь только визуальный размер/цвет. */
    icon-size: 18px;
}

/* ── Empty state ───────────────────────────────────────────────────────── */

.kb-chip-empty-icon {
    icon-size: 22px;
    color: var(--text-muted);
}

.kb-chip-empty-hint {
    color: var(--text-muted);
    font-size: 12px;
}

/* ── Footer ────────────────────────────────────────────────────────────── */

.kb-chip-footer-btn {
    background-color: transparent;
    color: var(--primary);
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.kb-chip-footer-btn:hover {
    background-color: var(--surface-hover);
}
