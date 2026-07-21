/* ───────────────────────── Template picker (модальное окно) ─────────
 * Всплывающее окно выбора шаблонов графа нод. Открывается кнопкой «+»
 * в баре вкладок node-editor'а. Слева — боковая панель разделов
 * (Генерация видео / Музыка / Аудио / Базовые / Свои), справа — сетка
 * карточек-превью (см. .ne-template-card ниже — классы перенесены из
 * бывшей templates_panel.mss).
 * ─────────────────────────────────────────────────────────────────── */

.tpl-picker-card {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    border-radius: 16px;
    width: 920px;
    height: 620px;
    overflow: hidden;
    box-shadow: 0 24px 60px rgba(10, 12, 24, 0.28);
}

/* ───────────────────────── Шапка ───────────────────────────────────── */

.tpl-picker-header {
    background-color: var(--bg-panel);
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.tpl-picker-title {
    color: var(--text);
    font-size: 16px;
    font-weight: 700;
}

.tpl-picker-action {
    color: var(--text-muted);
    background-color: transparent;
    border-radius: var(--radius-pill);
    padding: 6px 6px 6px 6px;
    icon-size: 18px;
    cursor: pointer;
    transition: color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}
.tpl-picker-action:hover {
    color: var(--text);
    background-color: var(--surface-hover);
}

/* ───────────────────────── Тело: sidebar + сетка ───────────────────── */

.tpl-picker-body {
    flex-grow: 1;
    background-color: var(--bg-panel);
}

.tpl-picker-sidebar {
    width: 210px;
    height: 100%;
    background-color: var(--bg-rail);
    border-right-width: 1px;
    border-color: var(--border-soft);
}

.tpl-picker-section {
    background-color: transparent;
    cursor: pointer;
    border-radius: 10px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}
.tpl-picker-section:hover {
    background-color: var(--surface-hover);
}
.tpl-picker-section--selected {
    background-color: var(--surface-selected);
}

.tpl-picker-section-icon {
    font-family: "Material Icons";
    font-size: 18px;
    color: var(--text-muted);
    transition: color var(--duration-fast) var(--ease-standard);
}
.tpl-picker-section--selected .tpl-picker-section-icon {
    color: var(--primary);
}

.tpl-picker-section-label {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
    transition: color var(--duration-fast) var(--ease-standard);
}
.tpl-picker-section--selected .tpl-picker-section-label {
    color: var(--primary-hover);
}

.tpl-picker-list {
    background-color: transparent;
    flex-grow: 1;
}

.tpl-picker-empty-title {
    color: var(--text-muted);
    font-size: 14px;
    font-weight: 500;
}
.tpl-picker-empty-hint {
    color: var(--text-subtle);
    font-size: 12px;
    text-align: center;
}

/* ───────────────────────── Карточка шаблона ────────────────────────── */

.ne-template-card {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    border-radius: 12px;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard),
                box-shadow var(--duration-fast) var(--ease-standard),
                transform var(--duration-fast) var(--ease-standard);
}
.ne-template-card:hover {
    background-color: var(--surface-hover);
    border-color: var(--border);
    box-shadow: 0 6px 18px rgba(20, 24, 40, 0.08);
}

.ne-template-card-preview {
    background-color: #1A1B22;
    border-radius: 8px;
    height: 88px;
    width: 100%;
    overflow: hidden;
}

.ne-template-card-name {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
    line-clamp: 1;
}

.ne-template-card-name-edit-host {
    background-color: transparent;
}

.ne-template-card-name-edit {
    background-color: var(--bg-shell);
    border-width: 1px;
    border-color: var(--primary);
    border-radius: 6px;
    color: var(--text);
    font-size: 13px;
    padding: 2px 6px 2px 6px;
}

.ne-template-card-desc-host {
    background-color: transparent;
}

.ne-template-card-desc {
    color: var(--text-muted);
    font-size: 11px;
    line-clamp: 2;
}

.ne-template-card-desc-empty {
    color: transparent;
    font-size: 1px;
    height: 0px;
}

/* ───────────────────────── Чипы (Full / Subgraph / Builtin) ───────── */

.ne-template-chip {
    color: var(--text-muted);
    background-color: var(--surface-hover);
    border-radius: var(--radius-pill);
    padding: 1px 8px 1px 8px;
    font-size: 10px;
    font-weight: 600;
}

.ne-template-chip--full {
    color: #1E40AF;
    background-color: #DBEAFE;
}
.ne-template-chip--subgraph {
    color: #6B21A8;
    background-color: #EDE9FE;
}

.ne-template-badge {
    color: var(--text-muted);
    background-color: var(--surface-selected);
    border-radius: var(--radius-pill);
    padding: 1px 8px 1px 8px;
    font-size: 10px;
    font-weight: 600;
}
