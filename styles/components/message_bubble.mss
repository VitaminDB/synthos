.msg-author {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
}

.msg-author-right {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
}

.msg-time {
    color: var(--text-muted);
    font-size: 12px;
}

.msg-bubble {
    padding: 10px 14px 10px 14px;
    max-width: 90%;
    box-shadow: 0 1px 2px rgba(17, 24, 39, 0.06);
}

.msg-bubble-in {
    /* `--surface-hover` светлее `--bg-chat` во всех тёмных темах и визуально
     * «приподнимает» бабл над фоном чата (в светлых — чуть off-white, что
     * тоже работает как нежная карточка). Использовать `--bg-shell` нельзя:
     * в тёмных темах он ТЕМНЕЕ `--bg-chat`, бабл выглядел «вдавленным». */
    background-color: var(--surface-hover);
    border-width: 1px;
    border-color: var(--border);
    border-radius: 4px 18px 18px 18px;
}

.msg-bubble-in .msg-text { color: var(--text); font-size: 14px; }

/* MarkdownView внутри ответа ассистента — все визуальные параметры через MSS.
 * Стандартные `color` / `font-size` задают body-текст, кастомные `--md-*` —
 * markdown-специфику (heading, link, code, quote, table, hr). */
.msg-bubble-md {
    color: var(--text);
    font-size: 14px;
    --md-heading-color: var(--text);
    --md-link-color: var(--primary);
    --md-code-bg: var(--surface-hover);
    --md-code-color: var(--primary);
    /* Чуть темнее чем сам бабл — code-блок виден как карточка, а тема
     * `InspiredGitHub` даёт читаемые цвета на светлом фоне. */
    --md-code-block-bg: var(--bg-window);
    --md-code-block-color: var(--text);
    /* Copy-кнопка поверх code-блока — подбор под светлый фон. */
    --md-copy-bg: rgba(0, 0, 0, 0.06);
    --md-copy-bg-hover: rgba(0, 0, 0, 0.14);
    --md-copy-color: var(--text-muted);
    --md-copy-flash-bg: rgba(34, 197, 94, 0.85);
    --md-quote-bg: var(--surface-hover);
    --md-quote-text-color: var(--text-muted);
    --md-quote-border-color: var(--primary);
    --md-bullet-color: var(--text-muted);
    --md-checkbox-color: var(--primary);
    --md-table-border-color: var(--border);
    --md-table-header-bg: var(--surface-hover);
    --md-table-header-color: var(--text);
    --md-table-stripe-bg: var(--surface-hover);
    --md-hr-color: var(--border);
    --md-strikethrough-color: var(--text-muted);
}

.msg-bubble-out {
    background-color: var(--primary);
    border-radius: 18px 4px 18px 18px;
}

.msg-bubble-out .msg-text { color: var(--on-primary); font-size: 14px; }

/* MarkdownView внутри исходящего бабла. Применяется поверх `.msg-bubble-md`
 * через двойной класс — переопределяем только те `--md-*`, где базовый
 * `--text`/`--primary` плохо читается на фоне `--primary` (белый-на-цветном).
 * Полупрозрачные оверлеи `rgba(255,255,255,…)` дают тонкие плашки кода и
 * цитат, не выпадая из «материального» вида. */
.msg-bubble-out-md {
    color: var(--on-primary);
    --md-heading-color: var(--on-primary);
    --md-link-color: var(--on-primary);
    --md-code-bg: rgba(255, 255, 255, 0.18);
    --md-code-color: var(--on-primary);
    --md-code-block-bg: rgba(0, 0, 0, 0.20);
    --md-code-block-color: var(--on-primary);
    --md-quote-bg: rgba(255, 255, 255, 0.12);
    --md-quote-text-color: var(--on-primary);
    --md-quote-border-color: var(--on-primary);
    --md-bullet-color: var(--on-primary);
    --md-checkbox-color: var(--on-primary);
    --md-table-border-color: rgba(255, 255, 255, 0.30);
    --md-table-header-bg: rgba(255, 255, 255, 0.18);
    --md-table-header-color: var(--on-primary);
    --md-table-stripe-bg: rgba(255, 255, 255, 0.10);
    --md-hr-color: rgba(255, 255, 255, 0.30);
    --md-strikethrough-color: rgba(255, 255, 255, 0.65);
    /* На primary-фоне дефолтный синий highlight выделения теряется —
     * дизайн-выбор: белый highlight 30% alpha сохраняет читаемость глифов. */
    selection-color: rgba(255, 255, 255, 0.30);
}

.msg-text {
    font-size: 14px;
}

/* Thinking-блок (chain-of-thought reasoning-моделей). Мини-карточка
 * внутри assistant-bubble, выше основного ответа. Заголовок кликабелен —
 * сворачивает/разворачивает тело. По умолчанию открыт пока стримит
 * (body пустой), сворачивается когда пошёл ответ. */
.msg-thinking {
    background-color: var(--surface-hover);
    border-width: 1px;
    border-color: var(--border-soft);
    border-radius: 10px;
    padding: 10px 14px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.msg-thinking:hover {
    background-color: var(--surface-hover);
}

.msg-thinking-title {
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 600;
}

.msg-thinking-icon {
    color: var(--primary);
    icon-size: 16px;
}

.msg-thinking-chevron {
    color: var(--text-muted);
    icon-size: 18px;
}

/* Тело размышлений — приглушённый цвет, чуть мельче чем основной ответ,
 * `--md-*` сдвинут под нейтральную палитру (без primary-акцентов) чтобы
 * не отвлекать от собственно ответа. */
.msg-thinking-body {
    color: var(--text-muted);
    font-size: 13px;
    --md-heading-color: var(--text);
    --md-link-color: var(--primary);
    --md-code-bg: var(--bg-shell);
    --md-code-color: var(--text-muted);
    --md-code-block-bg: var(--bg-window);
    --md-code-block-color: var(--text-muted);
    --md-quote-bg: var(--bg-shell);
    --md-quote-text-color: var(--text-muted);
    --md-quote-border-color: var(--border);
    --md-bullet-color: var(--text-muted);
    --md-hr-color: var(--border);
}

.msg-system {
    color: var(--text-muted);
    font-size: 13px;
    font-style: italic;
}

/* Вариант «system-плашки» с ошибкой (сервер недоступен, отмена, …). */
.msg-system-error {
    color: var(--error);
    font-size: 13px;
    font-style: italic;
}

/* Пульсирующие точки «ассистент печатает» — используется в пустом
 * assistant-placeholder’е пока пришёл хотя бы один токен. */
.msg-typing {
    color: var(--text-muted);
    font-size: 18px;
    letter-spacing: 2px;
    animation: typing-pulse 1.2s var(--ease-standard) infinite;
}

@keyframes typing-pulse {
    0%   { opacity: 0.35; transform: translateY(0); }
    50%  { opacity: 1.0;  transform: translateY(-1px); }
    100% { opacity: 0.35; transform: translateY(0); }
}

@keyframes bubble-pop-in {
    0%   { opacity: 0; transform: scale(0.92); }
    100% { opacity: 1; transform: scale(1); }
}

.msg-bubble {
    animation: bubble-pop-in var(--duration-med) var(--ease-standard);
}

/* ─── Прикреплённые к user-сообщению картинки ───────────────────────────
 * Каждая картинка — DecoratedBox `.msg-attachment-thumb` с фиксированной
 * шириной 280px и Image::Contain внутри. Скруглённые углы и тонкая рамка
 * подчёркивают что это вложение, а не часть текста. */
.msg-attachment-thumb {
    width: 280px;
    height: 200px;
    border-radius: 12px;
    background-color: rgba(255, 255, 255, 0.08);
    border-width: 1px;
    border-color: rgba(255, 255, 255, 0.18);
    overflow: hidden;
    transition: transform var(--duration-fast) var(--ease-standard);
}
.msg-attachment-thumb:hover { transform: scale(1.02); }

/* ─────────────────────────── Action toolbar ────────────────────────────── */
/* Тонкая полоска под ассистент-bubble. Сейчас единственное действие —
 * regenerate; место подготовлено для copy/edit и т.п. в будущем. */

.msg-actions {
    padding: 0;
}

/* Пустая заглушка во время `chat.pending=true` — Reactive показывает её
 * вместо настоящего toolbar'а, чтобы layout не «прыгал» при появлении/
 * исчезновении кнопок. Нулевая высота — занимаемое место незаметно. */
.msg-actions-empty {
    width: 0;
    height: 0;
    background-color: transparent;
}

/* Кнопка regenerate. Визуально неагрессивная по умолчанию (полу-прозрачная,
 * нейтральный muted-цвет), на hover оживает primary-акцентом. Та же
 * стилистика, что у `.chat-item-trailing-delete`, но без opacity:0 — на
 * каждое сообщение кнопка должна быть заметна, чтобы пользователь знал
 * о возможности перегенерации. */
ToolButton.msg-action-regen,
ToolButton.msg-action-copy {
    width: 28px;
    height: 28px;
    border-radius: 8px;
    background-color: transparent;
    color: var(--text-muted);
    icon-size: 16px;
    opacity: 0.55;
    transition:
        opacity var(--duration-fast) var(--ease-standard),
        background-color var(--duration-fast) var(--ease-standard),
        color var(--duration-fast) var(--ease-standard);
}

ToolButton.msg-action-regen:hover,
ToolButton.msg-action-copy:hover {
    background-color: var(--surface-hover);
    color: var(--primary);
    opacity: 1.0;
}

ToolButton.msg-action-regen:active,
ToolButton.msg-action-copy:active {
    background-color: var(--primary-soft);
}
