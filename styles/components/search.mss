/* ───────────────────────── Глобальный поиск ─────────────────────────
 * Пилюля в шапке чата (`search::trigger`) и раскрывающаяся панель
 * (`search::panel`). Панель — PopupPanel в overlay-слое, поэтому её фон и
 * тень задаются здесь, а не наследуются от страницы.
 * ─────────────────────────────────────────────────────────────────── */

/* ───────────────────────── Пилюля-триггер ───────────────────────── */

.search-trigger {
    background-color: var(--bg-search);
    border-width: 1px;
    border-color: var(--border);
    border-radius: var(--radius-pill);
    height: 34px;
    width: 380px;
    cursor: pointer;
    transition: border-color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}

.search-trigger:hover {
    border-color: var(--primary);
    background-color: var(--bg-shell);
}

.search-trigger-open {
    border-color: var(--primary);
    background-color: var(--bg-shell);
}

.search-trigger-icon {
    icon-size: 18px;
    color: var(--text-muted);
}

.search-trigger-text {
    font-size: 13px;
    color: var(--text-subtle);
}

/* ───────────────────────── Клавиши-подсказки ───────────────────────── */

.search-kbd {
    background-color: var(--bg-shell);
    border-width: 1px;
    border-color: var(--border);
    border-radius: 5px;
    padding: 2px 5px 2px 5px;
}

.search-kbd-iconbox {
    padding: 2px 4px 2px 4px;
}

.search-kbd-text {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-muted);
}

.search-kbd-icon {
    icon-size: 12px;
    color: var(--text-muted);
}

/* Место под «Enter» держится всегда — иначе строки прыгают по ширине,
 * когда курсор бежит по выдаче. */
.search-kbd-hidden {
    opacity: 0.0;
}

/* ───────────────────────── Панель ───────────────────────── */

.search-panel {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    border-radius: 16px;
    box-shadow: 0 24px 60px rgba(10, 12, 24, 0.28);
}

@keyframes search-card-in {
    0%   { opacity: 0.0; }
    100% { opacity: 1.0; }
}

/* Фон карточки — `--surface-hover`, а не `--bg-panel`: при
 * `window_opacity < 1` тема делает все `--bg-*` полупрозрачными, и сквозь
 * панель просвечивала лента чата — выдача становилась нечитаемой.
 * `--surface-hover` прозрачность окна не затрагивает, а тему — да. */
.search-card {
    background-color: var(--surface-hover);
    border-radius: 16px;
    animation: search-card-in var(--duration-fast) var(--ease-standard);
}

.search-input {
    background-color: var(--bg-search);
    color: var(--text);
    border-width: 1px;
    border-color: transparent;
    border-radius: 10px;
    height: 38px;
    font-size: 14px;
    icon-size: 18px;
    accent-color: var(--primary);
    transition: border-color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}

.search-input:focus {
    border-color: var(--primary);
    background-color: var(--bg-shell);
}

/* ───────────────────────── Чипы групп ───────────────────────── */

.search-chip {
    border-width: 1px;
    border-color: var(--border);
    border-radius: var(--radius-pill);
    background-color: transparent;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard);
}

.search-chip:hover {
    background-color: var(--surface-hover);
}

.search-chip-active {
    background-color: var(--primary-soft);
    border-color: var(--primary);
}

.search-chip-label {
    font-size: 11px;
    font-weight: 500;
    color: var(--text-muted);
}

.search-chip-label-active {
    color: var(--primary);
}

.search-chip-count {
    font-size: 10px;
    color: var(--text-subtle);
}

/* ───────────────────────── Плашка-заметка ─────────────────────────
 * Подсказка про раскладку («ghb,jh» → «прибор») и индикатор фонового
 * сканирования чатов. */

.search-note {
    background-color: var(--primary-soft);
    border-radius: 8px;
}

.search-note-icon {
    icon-size: 16px;
    color: var(--primary);
}

.search-note-text {
    font-size: 12px;
    color: var(--text-muted);
}

/* ───────────────────────── Выдача ───────────────────────── */

/* Бокс фиксированной высоты включается, когда строк больше VISIBLE_LINES —
 * иначе панель прыгала бы по высоте на каждый набранный символ. */
.search-results {
    height: 430px;
}

.search-group-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-subtle);
}

.search-group-count {
    font-size: 11px;
    color: var(--text-subtle);
}

.search-row {
    border-radius: 10px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.search-row:hover {
    background-color: var(--border);
}

.search-row-selected {
    background-color: var(--surface-selected);
}

.search-row-more-text {
    font-size: 12px;
    font-weight: 500;
    color: var(--primary);
}

.search-row-more-count {
    font-size: 11px;
    color: var(--text-subtle);
}

.search-row-sub {
    font-size: 11px;
    color: var(--text-muted);
}

.search-row-hint {
    font-size: 11px;
    color: var(--text-subtle);
}

/* ───────────────────────── Иконки строк ─────────────────────────
 * Цвет плитки — единственное, что отличает типы результатов на беглый
 * взгляд, поэтому у каждого свой тон из палитры темы. */

.search-icon-box {
    width: 30px;
    height: 30px;
    border-radius: 9px;
    background-color: var(--border);
}

/* Чаты — то, ради чего поиск открывают чаще всего: единственный тип с
 * акцентной плиткой. Остальные различаются цветом глифа. */
.search-icon-box-chat     { background-color: var(--primary-soft); }
.search-icon-box-more     { background-color: transparent; }

.search-row-icon {
    icon-size: 18px;
    color: var(--text-muted);
}

.search-row-icon-chat     { color: var(--primary); }
.search-row-icon-message  { color: var(--text-muted); }
.search-row-icon-page     { color: var(--presence-telegram); }
.search-row-icon-command  { color: var(--warning); }
.search-row-icon-model    { color: var(--presence-online); }
.search-row-icon-skill    { color: var(--presence-instagram); }
.search-row-icon-tool     { color: var(--presence-messenger); }
.search-row-icon-template { color: var(--text); }
.search-row-icon-more     { color: var(--primary); }

/* ───────────────────────── Пустая выдача ───────────────────────── */

.search-empty-icon {
    icon-size: 36px;
    color: var(--text-subtle);
}

.search-empty-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--text);
}

.search-empty-hint {
    font-size: 12px;
    color: var(--text-subtle);
}

/* ───────────────────────── Подвал ───────────────────────── */

.search-divider {
    color: var(--border-soft);
}

.search-footer-text {
    font-size: 11px;
    color: var(--text-subtle);
}
