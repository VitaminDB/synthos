/* Правая панель со списком скилов + редактор markdown. */

/* Содержимое правого сайдбара. Сам фон/padding идут от .settings-right —
 * здесь только прозрачный контейнер и типографика заголовка, совпадающая
 * с .settings-sidebar-title (26px, тот же отступ 12px по бокам). */
.skills-panel {
    background-color: transparent;
}

.skills-panel-header {
    background-color: transparent;
    padding: 0 12px 24px 12px;
}

.skills-panel-header-icon-wrap {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background-color: var(--primary-soft);
}

.skills-panel-header-icon {
    color: var(--primary);
    icon-size: 20px;
}

.skills-panel-header-title {
    font-size: 20px;
    font-weight: 700;
    color: var(--text);
}

.skills-list {
    background-color: var(--bg-panel);
    border-color: var(--border-soft);
}

/* Карточка одного скила в правой панели — зеркалит conversation_item
 * с главной страницы: паддинг, иконка слева, заголовок + preview. */

.skill-list-row {
    background-color: transparent;
    border-radius: 12px;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.skill-list-row:hover {
    background-color: var(--surface-hover);
}

.skill-list-row.selected {
    background-color: var(--surface-selected);
}

.skill-list-icon-wrap {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background-color: var(--primary-soft);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.skill-list-icon {
    color: var(--primary);
    icon-size: 18px;
}

.skill-list-title {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
}

.skill-list-subtitle {
    color: var(--text-muted);
    font-size: 13px;
}

/* Главная область редактирования скила. */

.skills-page {
    background-color: var(--bg-chat);
}

.skill-editor-header {
    background-color: var(--bg-shell);
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.skill-editor-badge {
    width: 44px;
    height: 44px;
    border-radius: 12px;
    background-color: var(--primary-soft);
}

.skill-editor-badge-icon {
    color: var(--primary);
    icon-size: 22px;
}

.skill-editor-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--text);
}

.skill-editor-subtitle {
    font-size: 13px;
    color: var(--text-muted);
}

/* Body редактора скила — структурный контейнер без своих визуальных
 * атрибутов: CodeEditor сам красит фон через `editor-bg` и заполняет
 * всю область, точно так же как `.code-editor-edit-body` на странице code. */
.skill-editor-body {
    padding: 0;
    background-color: transparent;
}

.skill-markdown-edit {
    background-color: transparent;
    font-size: 14px;
    color: var(--text);
    padding: 16px;
    font-family: monospace;
    transition: color var(--duration-med) var(--ease-standard);
}

.skill-markdown-edit .line-number {
    color: var(--text-subtle);
    background-color: transparent;
}

.skill-empty-bubble {
    width: 80px;
    height: 80px;
    border-radius: 999px;
    background-color: var(--primary-soft);
}

.skill-empty-icon {
    color: var(--primary);
    icon-size: 36px;
}

.skill-empty-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--text);
}

.skill-empty-text {
    font-size: 14px;
    color: var(--text-muted);
    text-align: center;
}

/* CodeEditor для редактора скила — все цвета/токены берутся из глобального
 * `CodeEditor {}` в base/reset.mss. Здесь оставляем класс пустым, чтобы
 * можно было точечно подкрутить геометрию через каскад при необходимости. */
.skill-code-editor {
    /* intentionally empty — наследует от глобального CodeEditor */
}

/* «+» в шапке списка скилов и edit/delete на каждой карточке. */
.skills-panel-add-btn {
    color: var(--primary);
    background-color: transparent;
    border-radius: 8px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.skills-panel-add-btn:hover {
    background-color: var(--surface-hover);
}

.skill-list-action {
    color: var(--text-muted);
    background-color: transparent;
    border-radius: 8px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.skill-list-action:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.skill-list-action.danger:hover {
    color: var(--error);
}

.skill-editor-action {
    color: var(--text-muted);
    background-color: transparent;
    border-radius: 10px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.skill-editor-action:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.skill-editor-action.danger:hover {
    color: var(--error);
}

/* CRUD-диалог скилов. Геометрия и большая часть цветов берётся из общих
 * `.code-editor-dialog-*` (см. styles/components/code_editor_dialogs.mss);
 * здесь — только то, что не покрывается глобальным `TextField {}` из
 * styles/base/reset.mss. TextField без локального .class() сразу попадает
 * в проектную тему — фон, рамка, accent, focus уже определены глобально. */
.skill-dialog-card {
    background-color: var(--bg-panel);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 24px 24px 20px 24px;
    min-width: 420px;
    max-width: 600px;
}

.skill-dialog-card.skill-dialog-danger {
    border-color: var(--error);
}

.skill-dialog-title {
    color: var(--text);
    font-size: 18px;
    font-weight: 600;
}

.skill-dialog-hint {
    color: var(--text-muted);
    font-size: 13px;
}

.skill-dialog-btn-primary {
    background-color: var(--primary);
    color: var(--on-primary);
    border-radius: var(--radius-panel);
    padding: 8px 16px;
    font-size: 13px;
    font-weight: 600;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.skill-dialog-btn-primary:hover {
    background-color: var(--primary-hover);
}

.skill-dialog-btn-secondary {
    background-color: transparent;
    color: var(--text);
    border-width: 1px;
    border-color: var(--border-strong);
    border-radius: var(--radius-panel);
    padding: 8px 16px;
    font-size: 13px;
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.skill-dialog-btn-secondary:hover {
    background-color: var(--surface-hover);
}

.skill-dialog-btn-danger {
    background-color: var(--error);
    color: var(--text-inverse);
    border-radius: var(--radius-panel);
    padding: 8px 16px;
    font-size: 13px;
    font-weight: 600;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.skill-dialog-btn-danger:hover {
    background-color: var(--error-hover);
}

.skill-dialog-empty {
    background-color: transparent;
    width: 0;
    height: 0;
}

/* Чипы скилов в правой панели — наследуют tools-active/available-chip,
 * добавляем лёгкое отличие иконки (psychology) и accent-цвет. */
.skills-chip {
    /* Геометрия и базовые цвета — от tools-active-chip / tools-available-chip.
     * Здесь — только override accent для иконки, чтобы визуально отличать
     * скилы от tools, не строя отдельный класс с нуля. */
    icon-color: var(--primary);
}
