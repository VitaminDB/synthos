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
