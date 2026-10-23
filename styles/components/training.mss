/* ─────────────────────── Страница «Обучение» ───────────────────────
 * Дообучение моделей адаптерами LoRA через службу нейросетей: левая
 * панель (задачи, датасеты, адаптеры), центр (обзор, форма, ход задачи,
 * редактор датасета, адаптер). Цвета — токены `styles/base/variables.mss`.
 * ───────────────────────────────────────────────────────────────── */

.training-page {
    background-color: var(--bg-shell);
}

.training-h-split {
    background-color: var(--bg-shell);
    border-color: var(--border-soft);
    accent-color: var(--primary);
    divider-thickness: 1px;
}

.training-center {
    background-color: var(--bg-shell);
}

.training-center-body {
    padding: 22px 28px 28px 28px;
    width: 100%;
    max-width: 980px;
}

/* ─── Левая панель ─────────────────────────────────────────────── */

.training-left {
    background-color: var(--bg-panel);
}

.training-left-body {
    padding: 8px 10px 16px 10px;
}

.training-left-section-row {
    padding: 14px 6px 4px 8px;
}

.training-left-section {
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.6px;
    text-transform: uppercase;
}

.training-left-empty {
    color: var(--text-subtle);
    font-size: 12px;
    padding: 4px 8px 6px 8px;
}

.training-item {
    padding: 8px 10px 8px 10px;
    border-radius: 10px;
    background-color: transparent;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.training-item:hover {
    background-color: var(--surface-hover);
}

.training-item.training-item-selected {
    background-color: var(--surface-selected);
}

.training-item-icon {
    font-size: 18px;
    color: var(--text-muted);
}

.training-item-selected .training-item-icon {
    color: var(--primary);
}

.training-item-title {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
}

.training-item-line {
    color: var(--text-muted);
    font-size: 11px;
}

.training-item-progress {
    height: 3px;
    border-radius: 2px;
    accent-color: var(--primary);
    background-color: var(--border-soft);
    margin: 4px 0 0 0;
}

/* ─── Карточки центра ──────────────────────────────────────────── */

.training-card {
    padding: 16px 18px 16px 18px;
    border-radius: var(--radius-panel);
    background-color: var(--bg-search);
    border-width: 1px;
    border-color: var(--border-soft);
}

.training-card-title {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
}

.training-section-title {
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.5px;
    text-transform: uppercase;
    padding: 6px 2px 0 2px;
}

.training-lead {
    color: var(--text);
    font-size: 14px;
    line-height: 1.5;
}

.training-hint {
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.45;
}

.training-warning {
    color: var(--warning);
    font-size: 12px;
}

.training-step-num {
    width: 30px;
    height: 30px;
    border-radius: 15px;
    background-color: var(--primary-soft);
}

.training-step-num-text {
    color: var(--primary);
    font-size: 14px;
    font-weight: 700;
}

/* ─── Форма ────────────────────────────────────────────────────── */

.training-form {
    padding: 4px 18px 4px 18px;
}

.training-field {
    padding: 12px 0 12px 0;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.training-field-label {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
}

.training-field-hint {
    color: var(--text-muted);
    font-size: 11px;
    line-height: 1.4;
}

.training-check-progress {
    width: 160px;
    height: 4px;
    accent-color: var(--primary);
    background-color: var(--border-soft);
}

/* ─── Кнопки ───────────────────────────────────────────────────── */

.training-btn {
    padding: 8px 14px;
    border-radius: 10px;
    background-color: transparent;
    border-width: 1px;
    border-color: var(--border-strong);
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard);
}

.training-btn:hover {
    background-color: var(--surface-hover);
}

.training-btn.training-btn-primary {
    background-color: var(--primary);
    border-color: var(--primary);
    color: var(--on-primary);
    font-weight: 600;
}

.training-btn.training-btn-primary:hover {
    background-color: var(--primary-hover);
    border-color: var(--primary-hover);
}

.training-btn.training-btn-danger {
    border-color: var(--error);
    color: var(--error);
}

.training-btn.training-btn-danger:hover {
    background-color: var(--error);
    color: var(--text-inverse);
}

.training-btn.training-btn-small {
    padding: 5px 10px;
    font-size: 12px;
}

.training-icon-btn {
    width: 30px;
    height: 30px;
    border-radius: 8px;
    color: var(--text-muted);
    background-color: transparent;
}

.training-icon-btn:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

/* ─── Ход задачи ───────────────────────────────────────────────── */

.training-progress {
    height: 6px;
    border-radius: 3px;
    accent-color: var(--primary);
    background-color: var(--border-soft);
}

.training-badge {
    padding: 3px 10px;
    border-radius: 999px;
    background-color: var(--border-soft);
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 600;
}

.training-badge.training-badge-run {
    background-color: var(--primary-soft);
    color: var(--primary);
}

.training-badge.training-badge-ok {
    background-color: var(--diff-added-bg);
    color: var(--success);
}

.training-badge.training-badge-error {
    background-color: var(--diff-removed-bg);
    color: var(--error);
}

.training-stat-value {
    color: var(--text);
    font-size: 18px;
    font-weight: 700;
}

.training-stat-label {
    color: var(--text-muted);
    font-size: 11px;
}

.training-chart {
    width: 100%;
    color: var(--text-muted);
    accent-color: var(--primary);
}

.training-kv-label {
    width: 150px;
    color: var(--text-muted);
    font-size: 12px;
}

.training-kv-value {
    color: var(--text);
    font-size: 12px;
}

/* ─── Ошибки и пустые состояния ────────────────────────────────── */

.training-error-bar {
    padding: 10px 16px 10px 16px;
    background-color: var(--diff-removed-bg);
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.training-error-icon {
    color: var(--error);
    font-size: 18px;
}

.training-error-text {
    color: var(--error);
    font-size: 12px;
}

.training-empty-icon {
    color: var(--text-subtle);
    font-size: 40px;
}

.training-empty-title {
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
}

/* ─── Редактор датасета ────────────────────────────────────────── *
 * Пример — спокойная карточка; реплики — стенограмма: круглый значок роли
 * слева, над текстом мелкая метка роли (щелчок меняет роль), сам текст —
 * без рамок и подложек. Цвет роли только у значка и метки. Вызовы
 * инструментов — тонкие плашки моноширинным шрифтом.
 * ───────────────────────────────────────────────────────────────── */

.dsx-body {
    padding: 20px 28px 32px 28px;
    width: 100%;
    max-width: 860px;
}

.dsx-card {
    padding: 14px 16px 12px 16px;
    border-radius: 12px;
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border-soft);
}

.dsx-num {
    color: var(--text-subtle);
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 12px;
    font-weight: 600;
}

.dsx-pill {
    padding: 1px 8px;
    border-radius: 999px;
    background-color: var(--surface-hover);
}

.dsx-pill-text {
    color: var(--text-muted);
    font-size: 11px;
}

.dsx-pill.dsx-pill-tool {
    background-color: var(--warning-soft);
}

.dsx-pill-tool .dsx-pill-text {
    color: var(--warning);
}

.dsx-icon {
    width: 26px;
    height: 26px;
    border-radius: 7px;
    color: var(--text-subtle);
    background-color: transparent;
}

.dsx-icon:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.dsx-muted {
    color: var(--text-muted);
    font-size: 11px;
}

/* Реплика */

.dsx-turn {
    padding: 0;
}

.dsx-avatar {
    width: 26px;
    height: 26px;
    border-radius: 13px;
    background-color: var(--surface-hover);
    margin: 2px 0 0 0;
}

.dsx-avatar.dsx-avatar-assistant {
    background-color: var(--primary-soft);
}

.dsx-avatar.dsx-avatar-system {
    background-color: var(--warning-soft);
}

.dsx-glyph {
    font-family: "Material Icons";
    font-size: 15px;
    color: var(--text-muted);
}

.dsx-glyph.dsx-glyph-assistant { color: var(--primary); }
.dsx-glyph.dsx-glyph-system { color: var(--warning); }
.dsx-glyph.dsx-glyph-tool { color: var(--avatar-violet); }
.dsx-glyph.dsx-glyph-user { color: var(--avatar-blue); }
.dsx-glyph.dsx-glyph-tools { color: var(--warning); }

/* Метка роли — кнопка-текст: мелко, заглавными, цветом роли. */
.dsx-role {
    padding: 3px 6px;
    border-radius: 6px;
    border-width: 0;
    background-color: transparent;
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.6px;
    text-transform: uppercase;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.dsx-role:hover {
    background-color: var(--surface-hover);
}

.dsx-role.dsx-role-user { color: var(--avatar-blue); }
.dsx-role.dsx-role-assistant { color: var(--primary); }
.dsx-role.dsx-role-system { color: var(--warning); }
.dsx-role.dsx-role-tool { color: var(--avatar-violet); }
.dsx-role.dsx-role-tools { color: var(--warning); padding: 0; }

/* Текст реплики — без рамки и фона, выделяется только фокусом. */
.dsx-text {
    padding: 0;
}

.dsx-text MultilineTextEdit {
    background-color: transparent;
    border-width: 0;
    border-radius: 6px;
    padding: 2px 6px;
    color: var(--text);
    font-size: 13px;
    line-height: 1.55;
}

.dsx-text MultilineTextEdit:focus {
    background-color: var(--surface-hover);
}

.dsx-text.dsx-mono MultilineTextEdit {
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 12px;
    color: var(--text-muted);
}

/* Имя функции — тихое поле моноширинным. */
.dsx-name TextField {
    height: 24px;
    padding: 0 8px;
    border-radius: 6px;
    border-width: 0;
    background-color: var(--surface-hover);
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 12px;
    color: var(--text);
}

/* Вызов инструмента */

.dsx-call {
    padding: 6px 8px 6px 8px;
    margin: 4px 0 2px 6px;
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border-soft);
    background-color: var(--bg-shell);
}

.dsx-call-f {
    color: var(--avatar-violet);
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 13px;
    font-weight: 700;
    padding: 0 2px 0 4px;
}

/* Описания инструментов */

.dsx-tools {
    padding: 10px 12px 10px 12px;
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    background-color: var(--bg-shell);
}

/* Тихие ссылки: «+ Пользователь · Ассистент · …», «Вызов», страницы. */
.dsx-add-row {
    padding: 2px 0 0 32px;
}

.dsx-add-icon {
    font-family: "Material Icons";
    font-size: 14px;
    color: var(--text-subtle);
    padding: 0 2px 0 0;
}

.dsx-link {
    padding: 3px 8px;
    border-radius: 6px;
    border-width: 0;
    background-color: transparent;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.dsx-link:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.dsx-pager {
    padding: 6px 0 0 0;
}

.training-pager {
    padding: 4px 0 0 0;
}

/* ─── Карточка «Адаптеры LoRA» в чате ──────────────────────────── */

.lora-chat-body {
}

.lora-chat-row {
    padding: 8px 0 4px 0;
    border-top-width: 1px;
    border-color: var(--border-soft);
}

/* Выключенный переключатель на карточке: дорожка темнее фона карточки
 * (общий `Toggle` красится в --bg-search — цвет самой карточки). */
.training-toggle {
    background-color: var(--border-strong);
}
