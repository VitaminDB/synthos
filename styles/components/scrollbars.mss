/* Глобальная тема scrollbar для всех виджетов со встроенной прокруткой.
 *
 * Применяется к ScrollView, Page, TreeView, ListView, TableView, CodeEditor
 * и Terminal — общий компонент `syngui::widgets::scroll::scrollbar` читает
 * эти MSS-properties через `MssFields::scrollbar_style(fg)`.
 *
 * Дизайн-цели:
 * - Тонкий (8px) thumb с автоскрытием — не отвлекает от контента.
 * - Цвет — приглушённый text-subtle, hover/drag — насыщеннее (text-muted).
 * - Track — прозрачный, видно только при hover/drag.
 * - Радиус скругления = ширина / 2 (pill-shape).
 *
 * Per-widget overrides ниже задают узкий вариант для side-panels (TreeView в
 * code editor, conversations list) и более компактный для tooltip-scroll'ов. */

ScrollView,
Page,
TreeView,
ListView,
TableView,
CodeEditor,
Terminal {
    scrollbar-width: 8px;
    scrollbar-radius: 4px;
    scrollbar-color: var(--text-subtle);
    scrollbar-thumb-hover-color: var(--text-muted);
    scrollbar-track-color: rgba(28, 30, 38, 0.06);
    scrollbar-policy: auto;
    scrollbar-fade-delay: 1.5;
}

/* Узкий 6px scrollbar для боковых панелей с плотным деревом/списком —
 * file-tree code editor'а и список бесед. Текст рядом и так визуально
 * перегружает узкую колонку, толстый scrollbar только усугубит. */
.code-editor-tree,
.conv-list,
.chat-list {
    scrollbar-width: 6px;
    scrollbar-radius: 3px;
}

/* Особый кейс — `tool_messages.mss` уже использовал `scrollbar-width: 8px`
 * для `.tool-confirm-args-scroll`. Это значение совпадает с глобальным,
 * поэтому отдельных правил здесь не нужно. */

/* Code editor / Terminal — слегка ярче thumb (контент тёмный, нужен
 * заметный контраст). */
CodeEditor,
Terminal {
    scrollbar-color: rgba(156, 163, 175, 0.50);
    scrollbar-thumb-hover-color: rgba(156, 163, 175, 0.85);
}
