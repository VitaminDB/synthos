/* Syn-чат: специфичные стили поверх chat_* и right_panel. Большинство
   классов (`.chats-column`, `.chat-pane-wrap`, `.message-area`, `.msg-*`,
   `.input-*`) переиспользуются из llama-chat без изменений. */

/* ─────────────── Правая панель ─────────────── */

.syn-chat-right {
  /* Ширина — от правого SplitView (`SynChatCtx.right_split_ratio`);
     минимальная ширина гарантируется `min_size(240)` в Rust. */
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
  font-size: 12px;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

/* ── Карточка модели + sampling + контекст + system ── */

.sampling-card {
  padding: 12px;
  border-radius: 10px;
  background-color: var(--bg-elev-1);
  border: 1px solid var(--border-soft);
}

.sampling-row {
  padding: 4px 0;
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

.right-pick-btn {
  padding: 6px 10px;
  border-radius: 6px;
  background-color: var(--primary);
  color: var(--on-primary);
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

.right-unload-btn-full {
  background-color: var(--error-container);
  color: var(--on-error-container);
  border-radius: 8px;
  padding: 8px 14px;
  height: 36px;
  font-weight: 600;
  transition: background-color 140ms ease-out, transform 140ms ease-out;
}

.right-unload-btn-full:hover {
  background-color: var(--error);
  color: var(--on-error);
  transform: translateY(-1px);
}

.right-unload-btn-full:disabled {
  opacity: 0.4;
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

/* ── Details — метрики ── */

.details-metric-row {
  padding: 4px 0;
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

.chat-header-title-wrap {
  cursor: pointer;
}

.chat-header-title-edit {
  font-size: 15px;
  font-weight: 600;
  background-color: var(--bg-window);
  border: 1px solid var(--primary);
  border-radius: 4px;
  padding: 2px 6px;
}

.chat-header-empty {
  padding: 10px 16px;
  color: var(--text-muted);
}

.chat-header-empty-icon {
  font-size: 20px;
}

.chat-header-empty-text {
  font-size: 13px;
  color: var(--text-muted);
}

.chat-header-active {
  padding: 8px 16px;
}

.chat-header-subtitle {
  font-size: 11px;
  color: var(--text-muted);
}
