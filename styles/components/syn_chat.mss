/* Syn-чат: специфичные стили поверх chat_* и right_panel. Большинство
   классов (`.chats-column`, `.chat-pane-wrap`, `.message-area`, `.msg-*`,
   `.input-*`) переиспользуются из llama-chat без изменений. */

/* ─────────────── Правая панель ─────────────── */

.syn-chat-right {
  /* Ширина — от правого SplitView (`SynChatCtx.right_split_ratio`);
     минимальная ширина гарантируется `min_size(240)` в Rust. */
  background-color: var(--bg-panel);
}

.syn-chat-left {
  /* Инструменты/скилы слева — тот же вид, что у правой панели. */
  background-color: var(--bg-panel);
}

/* ─────────────── Drag-разделители (IDE-стиль) ─────────────── */

/* Hit-area 6px задаётся `divider_width(6.0)` в Rust, визуальная полоска —
   1px: выглядит как обычный border между панелями, но тянется мышью.
   Hover/drag подсветка — встроенная логика SplitView (`accent-color`).
   Единый стиль с `.code-editor-h-split` / `.syn-explorer-h-split`. */
.syn-chat-h-split {
  border-color: var(--border-soft);
  accent-color: var(--primary);
  divider-thickness: 1px;
}

.right-panel-section-title {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

/* ── Сворачиваемые карточки боковых панелей (`components::collapsible_card`) ──
 * Шапка одна на обе панели: иконка в акценте, заголовок ужимается до
 * остатка и обрезается в одну строку (кнопки справа держат размер), шеврон
 * — тот же, что у карточек «Деталей». Отступ под шапкой — у тела, чтобы у
 * свёрнутой карточки его не было. */
.collapsible-card-icon {
  icon-size: 16px;
  color: var(--primary);
}

.collapsible-card-title {
  flex-grow: 1;
  min-width: 0;
}

.collapsible-card-chevron {
  icon-size: 18px;
  color: var(--text-subtle);
}

.collapsible-card-body {
  padding-top: 8px;
}

.tools-section .collapsible-card-body {
  padding-top: 10px;
}

/* ── Карточка модели + sampling + контекст + system ── */

/* Панель узкая и высокая: карточки идут стопкой, поэтому лишний padding
 * здесь стоит дорого — системный prompt внизу выдавливало за край. */
.sampling-card {
  padding: 10px;
  border-radius: 10px;
  background-color: var(--bg-search);
  border: 1px solid var(--border-soft);
}

.sampling-row {
  padding: 1px 0;
}

.sampling-label {
  font-size: 12px;
  color: var(--text);
  font-weight: 500;
}

.sampling-value {
  font-size: 12px;
  color: var(--text-muted);
  font-family: monospace;
}

/* Подсказка вместо пресетов, пока модель не выбрана. */
.sampling-hint {
  font-size: 11px;
  color: var(--text-muted);
}

/* «По умолчанию / Свои» — в размер пикеров карточки, а не 36px кнопки. */
.sampling-mode-switch {
  width: 100%;
  height: 28px;
  font-size: 12px;
  border-radius: 6px;
}

/* Пресет модели и глубина размышлений — на всю ширину карточки, как
 * пикер системного промпта. */
.sampling-preset-picker {
  width: 100%;
  height: 28px;
  font-size: 12px;
  border-radius: 6px;
  border: 1px solid var(--border-soft);
  background-color: var(--bg-window);
  color: var(--text);
}

.system-prompt-edit {
  font-size: 12px;
  border-radius: 6px;
  border: 1px solid var(--border-soft);
  background-color: var(--bg-window);
}

/* ── Статус модели ── */

.model-status {
  padding: 10px;
  border-radius: 8px;
  border: 1px solid var(--border-soft);
}

.model-status.loading {
  background-color: rgba(140, 180, 250, 0.08);
  border-color: rgba(140, 180, 250, 0.30);
}

.model-status.error {
  background-color: rgba(230, 100, 100, 0.08);
  border-color: rgba(230, 100, 100, 0.30);
}

.model-status.ready {
  background-color: rgba(120, 200, 140, 0.08);
  border-color: rgba(120, 200, 140, 0.30);
}

.model-status.idle {
  background-color: var(--bg-window);
}

.model-status-icon {
  font-size: 20px;
  color: var(--text-muted);
}

.model-status-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.model-status-subtitle {
  font-size: 11px;
  color: var(--text-muted);
}

/* ── Кнопки выбора / выгрузки модели ── */

.right-pick-row {
  padding-top: 4px;
}

/* Picker стоит в одной строке с load/unload, поэтому высота и радиус
 * совпадают с `.right-model-btn` — иначе кнопки в строке разного роста. */
.right-pick-btn {
  width: 36px;
  height: 36px;
  border-radius: 8px;
  background-color: var(--primary);
  color: var(--on-primary);
  icon-size: 18px;
  transition: background-color 140ms ease-out, transform 140ms ease-out;
}

.right-pick-btn:hover {
  background-color: var(--primary-hover);
  transform: translateY(-1px);
}

.right-pick-btn.disabled {
  opacity: 0.5;
}

.right-unload-btn {
  padding: 6px;
  border-radius: 6px;
  color: var(--text-muted);
}

.right-unload-btn.disabled {
  opacity: 0.3;
}

/* Кнопка загрузки/выгрузки модели.
 *
 * Раньше здесь стояли `var(--error-container)` / `var(--on-error-container)`
 * — таких переменных в палитре нет вовсе (см. `styles/base/variables.mss`),
 * поэтому фон резолвился в ничто, и кнопка выглядела полностью плоской.
 * Плюс «Загрузить» и «Выгрузить» делили один класс, хотя это действия
 * разного веса: загрузка — нейтральная, выгрузка — деструктивная.
 *
 * Общая геометрия здесь, цвета — в модификаторах ниже. Border есть у обеих:
 * на тёмном фоне панели заливка сама по себе почти не читается. */
.right-model-btn {
  border-radius: 8px;
  border-width: 1px;
  padding: 8px 14px;
  height: 36px;
  font-weight: 600;
  transition:
    background-color 140ms ease-out,
    border-color 140ms ease-out,
    transform 140ms ease-out;
}

.right-model-btn:disabled {
  opacity: 0.4;
}

.right-model-btn-load {
  background-color: var(--surface-hover);
  border-color: var(--border-strong);
  color: var(--text);
}

.right-model-btn-load:hover {
  background-color: var(--primary-soft);
  border-color: var(--primary);
  color: var(--text);
  transform: translateY(-1px);
}

.right-model-btn-unload {
  background-color: transparent;
  border-color: var(--error);
  color: var(--error);
}

/* Белый, а не `--text-inverse`: в тёмных темах inverse — тёмный цвет
 * (он для текста на светлой подложке), а здесь заливка красная. */
.right-model-btn-unload:hover {
  background-color: var(--error);
  border-color: var(--error);
  color: #FFFFFF;
  transform: translateY(-1px);
}

.right-reset-wrap {
  padding-top: 4px;
}

.right-reset-btn {
  padding: 6px 10px;
  border-radius: 6px;
  color: var(--text-muted);
  border: 1px solid var(--border-soft);
}

/* ── Chat-header кнопки очистки / удаления ── */

.chat-header-action {
  padding: 4px;
  border-radius: 6px;
  color: var(--text-muted);
}

.chat-header-action:hover {
  background-color: var(--surface-hover);
  color: var(--text);
}


/* ── Карточка «Система»: библиотека промптов и окно редактора ──
 * Шапка карточки: заголовок + четыре маленькие ToolButton (создать /
 * переименовать / удалить / открыть в окне). Под ней — дропдаун пресетов на
 * всю ширину, ниже — компактный редактор. Плавающее окно
 * (`pages::syn_chat::prompt_window`) — редактор на всю площадь, растёт
 * вместе с окном через flex-grow. */

.system-prompt-action {
  padding: 3px;
  border-radius: 6px;
  color: var(--text-muted);
}

.system-prompt-action:hover {
  background-color: var(--surface-hover);
  color: var(--text);
}

.system-prompt-picker {
  width: 100%;
  height: 28px;
  font-size: 12px;
  border-radius: 6px;
  border: 1px solid var(--border-soft);
  background-color: var(--bg-window);
  color: var(--text);
}

.system-prompt-window {
  background-color: var(--bg-panel);
  color: var(--text);
  border: 1px solid var(--border-strong);
  border-radius: 10px;
  padding: 12px;
  min-width: 420px;
  min-height: 280px;
  font-size: 13px;
}

.system-prompt-window-edit {
  flex-grow: 1;
  font-size: 13px;
  border-radius: 8px;
  border: 1px solid var(--border-soft);
  background-color: var(--bg-window);
  color: var(--text);
}

.system-prompt-window-stats {
  font-size: 11px;
  color: var(--text-subtle);
}
