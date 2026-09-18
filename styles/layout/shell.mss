/* Transparent backdrop around the rounded shell, plus the shell itself.
 *
 * Padding и rounded переключаются по window-state через syngui pseudo-class
 * `:window-maximized`. В restored — окно «парит» с тенью и 30px воздуха
 * вокруг для resize-grab. В maximize — padding/тень/border обнуляются с
 * плавным transition (200ms), окно занимает весь экран без зазоров. */

.window-backdrop {
    background-color: #00000000;
    padding: 30px;
    transition: padding 200ms ease;
}

.window-backdrop:window-maximized {
    padding: 0;
}

.shell {
    background-color: var(--bg-shell);
    border-radius: var(--radius-shell);
    border-width: 1px;
    border-color: var(--border);
    box-shadow: 0 12px 36px rgba(24, 24, 43, 0.12);
    transition: border-radius 200ms ease, box-shadow 200ms ease, border-width 200ms ease;
}

.shell:window-maximized {
    border-radius: 0;
    border-width: 0;
    box-shadow: none;
}

.titlebar {
    background-color: var(--bg-shell);
    padding: 0 0 0 16px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
    cursor: default;
    height: 32px;
}

.titlebar-title {
    color: var(--text-muted);
    font-size: 12px;
}

/* Пилюли рядом с названием: стадия релиза и номер сборки (`pkgrel`).
 * Мельче и приглушённее самого заголовка — это служебная метка, а не
 * часть имени приложения. */
/* Высота задана явно: с ней `Center` внутри имеет что центрировать, и
 * подпись стоит ровно по середине пилюли независимо от того, есть ли у
 * глифов выносные элементы. */
.titlebar-badge {
    height: 16px;
    padding: 0 7px;
    border-radius: var(--radius-pill);
    border-width: 1px;
}

.titlebar-badge-text {
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.6px;
    text-transform: uppercase;
}

.titlebar-badge-stage {
    background-color: var(--primary-soft);
    border-color: var(--primary);
}

.titlebar-badge-stage .titlebar-badge-text {
    color: var(--primary);
}

/* Номер сборки — моноширинный и без разрядки: это число, а не слово. */
.titlebar-badge-build {
    background-color: var(--bg-search);
    border-color: var(--border-soft);
}

.titlebar-badge-build .titlebar-badge-text {
    color: var(--text-subtle);
    font-family: monospace;
    font-weight: 600;
    letter-spacing: 0;
    text-transform: none;
}

/* Чипы-ссылки Donate / GitHub у кнопок окна — без рамки и подложки, как
 * сами кнопки: подпись `.titlebar-badge-text` с иконкой, подложка только
 * при наведении (`:hover` самого чипа, с переходом). Цвет иконки и подписи —
 * класс `--hover`, который ставит `titlebar::link_chip`: hover у потомка
 * считается по его собственным границам, не по чипу. */
.titlebar-chip {
    height: 20px;
    padding: 0 8px 0 7px;
    border-radius: var(--radius-pill);
    background-color: transparent;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.titlebar-chip:hover {
    background-color: var(--surface-hover);
}

/* Полупрозрачная роза, а не токен: читается и на светлых, и на тёмных темах. */
.titlebar-chip.titlebar-chip-donate:hover {
    background-color: rgba(229, 72, 122, 0.12);
}

.titlebar-chip-text {
    color: var(--text-muted);
}

.titlebar-chip-icon {
    color: #E5487A;
    icon-size: 11px;
}

.titlebar-chip-logo {
    width: 11px;
    height: 11px;
    color-tint: var(--text-muted);
}

/* Прогресс загрузок HuggingFace перед чипами-ссылками: иконка, полоска,
 * «23% · 30 МБ/с». Подложка постоянная — это статус, а не ссылка. */
.titlebar-dl-empty {
    width: 0;
    height: 0;
}

.titlebar-chip.titlebar-dl {
    background-color: var(--primary-soft);
    margin-right: 6px;
}

.titlebar-chip.titlebar-dl:hover {
    background-color: var(--surface-selected);
}

.titlebar-dl-icon {
    icon-size: 12px;
    color: var(--primary);
}

.titlebar-dl-rail {
    width: 72px;
    height: 4px;
    border-radius: 2px;
    background-color: rgba(148, 163, 184, 0.35);
    overflow: hidden;
}

.titlebar-dl-fill {
    height: 4px;
    border-radius: 2px;
    background-color: var(--primary);
    transition: width var(--duration-fast) var(--ease-standard);
}

.titlebar-dl-text {
    font-size: 10px;
    font-weight: 600;
    font-family: monospace;
    color: var(--text);
}

.titlebar-dl--idle .titlebar-dl-icon { color: var(--warning); }
.titlebar-dl--idle .titlebar-dl-fill { background-color: var(--warning); }

.titlebar-chip--hover .titlebar-chip-text   { color: var(--text); }
.titlebar-chip--hover .titlebar-chip-logo   { color-tint: var(--text); }
.titlebar-chip-donate.titlebar-chip--hover .titlebar-chip-text { color: #E5487A; }

.window-control {
    width: 44px;
    height: 32px;
    background-color: transparent;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.window-control-icon {
    color: var(--text);
    icon-size: 16px;
}

.window-control:hover           { background-color: var(--surface-hover); }
.window-control.close:hover     { background-color: #E81123; }
.window-control.close:hover .window-control-icon { color: #FFFFFF; }

/* ───────────────────────── Mini-статусбар ──────────────────────────
 * Узкая декоративная полоса (4px) внизу окна. Информации не несёт —
 * нужна, чтобы окно «закруглялось» снизу аккуратной завершающей
 * линией, а не обрезалось 1px-border'ом панелей контента.
 * ─────────────────────────────────────────────────────────────────── */
.window-statusbar {
    background-color: var(--bg-rail);
    border-top-width: 1px;
    border-color: var(--border-soft);
    height: 4px;
}
