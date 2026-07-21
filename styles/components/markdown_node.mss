/* Декоративная markdown-нода редактора нод.
 *
 * Полупрозрачность — через `background-color: rgba(...)` на карточке и
 * `.node-card-body`, а не `opacity` на родителе: opacity блекнет ВЕСЬ
 * контент (включая текст и заголовки), а rgba-фон оставляет текст
 * 100% непрозрачным и читаемым. Эффект «полупрозрачная нода» сохранён —
 * grid редактора нод просвечивает через карточку.
 *
 * `TransformBox` (resize-режим) кастомизируется через `--tb-*` переменные.
 * `MarkdownView` поддерживает только `--md-*` переменные (никаких MSS-классов
 * на внутренних элементах — все headings/code/blockquote/list рисуются
 * напрямую в DisplayList). Задаём полный набор под тёмную палитру
 * node-editor, чтобы заголовки/код/цитаты/ссылки/таблицы читались
 * на тёмном фоне.
 */

.node-card.markdown-node {
    /* Полупрозрачный тёмный фон через переопределение `--node-bg`
       (определена в `.node-card`). grid редактора просвечивает через
       карточку. Если пользователь задаст tint через ContextMenu —
       inline-style `--node-bg` переопределит это значение, hover
       background-color не мигает. */
    --node-bg: rgba(42, 45, 56, 0.4);
}

/* Body внутри markdown-ноды — тоже полупрозрачно, чтобы grid просвечивал
   через всю карточку, а не только через header. Padding отделяет
   текст/редактор от рамки карточки и оставляет место под `TransformBox`
   selection-handles в режиме resize. */
.markdown-node .node-card-body {
    background-color: rgba(35, 38, 49, 0.4);
    padding: 12px;
}

/* ── TransformBox handle'ы под палитру node-editor ───────────────────── */
.markdown-node {
    --tb-border-color: var(--primary);
    --tb-border-width: 1.5px;
    --tb-handle-size: 12px;
    --tb-handle-color: #2A2D38;
    --tb-handle-border-color: var(--primary);
}

/* ── Полная палитра MarkdownView для тёмной темы ─────────────────────── */
.markdown-node {
    /* Базовый параграфный текст. */
    color: #E4E7EE;
    font-size: 13px;
    line-height: 1.55;

    /* Заголовки h1..h6 — светлые, с явной иерархией размеров. */
    --md-heading-color: #F5F7FB;
    --md-h1-size: 20px;
    --md-h2-size: 17px;
    --md-h3-size: 15px;
    --md-h4-size: 14px;
    --md-h5-size: 13px;
    --md-h6-size: 13px;
    --md-heading-spacing: 6px;

    /* Inline `code`: розовый текст на тёмно-белесом полупрозрачном фоне. */
    --md-code-bg: rgba(255, 255, 255, 0.08);
    --md-code-color: #FCA5A5;
    --md-code-font-size: 12px;
    --md-code-padding-h: 5px;
    --md-code-radius: 4px;

    /* Code block: глубокий тёмный фон, отделённый от карточки. */
    --md-code-block-bg: rgba(0, 0, 0, 0.35);
    --md-code-block-color: #E4E7EE;
    --md-code-block-radius: 6px;
    --md-code-block-padding: 10px;

    /* Blockquote: лёгкая «вкладка» с акцентной полосой слева. */
    --md-quote-bg: rgba(255, 255, 255, 0.04);
    --md-quote-text-color: #C5C9D2;
    --md-quote-border-color: var(--primary);
    --md-quote-border-width: 3px;
    --md-quote-padding-left: 10px;
    --md-quote-padding-v: 6px;
    --md-quote-radius: 4px;

    /* Ссылки — primary-цвет, обычный для тёмной темы. */
    --md-link-color: #F19A87;

    /* Списки. */
    --md-list-indent: 18px;
    --md-bullet-color: #94A3B8;
    --md-checkbox-color: #94A3B8;
    --md-checkbox-check-color: var(--primary);

    /* Таблицы. */
    --md-table-border-color: rgba(255, 255, 255, 0.14);
    --md-table-header-bg: rgba(255, 255, 255, 0.08);
    --md-table-header-color: #F5F7FB;
    --md-table-stripe-bg: rgba(255, 255, 255, 0.03);

    /* Горизонтальная черта. */
    --md-hr-color: rgba(255, 255, 255, 0.14);
    --md-hr-thickness: 1px;

    /* Прочее. */
    --md-strikethrough-color: #94A3B8;
    --md-image-placeholder-bg: rgba(255, 255, 255, 0.06);
    --md-image-placeholder-color: #94A3B8;
    --md-footnote-color: #94A3B8;
    --md-footnote-divider-color: rgba(255, 255, 255, 0.10);

    /* Copy-code кнопка (отображается при .with_copy_code(true)). */
    --md-copy-bg: rgba(255, 255, 255, 0.08);
    --md-copy-bg-hover: rgba(255, 255, 255, 0.18);
    --md-copy-color: #E4E7EE;
    --md-copy-flash-bg: rgba(34, 197, 94, 0.30);
    --md-copy-radius: 4px;
    --md-copy-size: 14px;
    --md-copy-margin: 6px;
}
