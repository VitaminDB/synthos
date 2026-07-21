/* Tool-call / tool-result bubbles + диалог подтверждения.
 * Все значения через MSS-переменные — тема меняет цвета централизованно.
 * Анимации: transition на hover-эффектах, pop-in у карточек. */

/* ───────────────────────── Tool-call bubble ───────────────────────── */

.tool-call-card {
    background-color: var(--bg-panel);
    border-radius: 14px;
    border-width: 1px;
    border-color: var(--border);
    padding: 12px 14px;
    /* Ограничиваем ширину, чтобы длинные JSON-аргументы не выталкивали
     * карточку за правый край ленты (тот же паттерн, что у `.msg-bubble`). */
    max-width: 90%;
    transition: border-color var(--duration-med) var(--ease-standard),
                background-color var(--duration-med) var(--ease-standard);
    animation: bubble-pop-in var(--duration-med) var(--ease-standard);
}

.tool-call-card:hover {
    border-color: var(--primary);
    background-color: var(--surface-hover);
}

.tool-call-icon {
    color: var(--primary);
    font-size: 18px;
}

.tool-call-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}

.tool-call-hint {
    font-size: 12px;
    color: var(--text-subtle);
}

.tool-call-args-wrap {
    background-color: var(--bg-search);
    border-radius: 10px;
    padding: 10px 12px;
    border-width: 1px;
    border-color: var(--border-soft);
}

.tool-call-args {
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 12px;
    color: var(--text-muted);
    white-space: pre;
}

/* ───────────────────────── Tool-result bubble ──────────────────────── */

/* Квадратная «аватар»-плашка слева от карточки tool-result — визуально
 * отличает результат инструмента от обычного сообщения ассистента, при
 * этом сохраняя ту же сетку (avatar + meta), что даёт bounded-width
 * для `max-width: 90%` на карточке. */
.tool-result-avatar {
    width: 32px;
    height: 32px;
    border-radius: 8px;
    background-color: var(--primary-soft);
}

.tool-result-avatar-icon {
    color: var(--primary);
    icon-size: 18px;
}

.tool-result-card {
    background-color: var(--bg-panel);
    border-radius: 14px;
    border-width: 1px;
    border-color: var(--border);
    padding: 12px 14px;
    /* Длинный `stdout` (например, `ls -la`) мог раздувать bubble до всей
     * ширины viewport. Ограничиваем, чтобы Text-виджет внутри начал
     * wrap’иться по available_width. */
    max-width: 90%;
    transition: border-color var(--duration-med) var(--ease-standard);
    animation: bubble-pop-in var(--duration-med) var(--ease-standard);
}

.tool-result-card-error {
    border-color: var(--error);
    background-color: rgba(229, 83, 83, 0.05);
}

.tool-result-icon {
    color: var(--text-muted);
    font-size: 18px;
}

.tool-result-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}

.tool-result-status-icon {
    color: var(--presence-online);
    font-size: 18px;
}

.tool-result-status-icon-error {
    color: var(--error);
}

.tool-result-body-wrap {
    background-color: var(--bg-search);
    border-radius: 10px;
    padding: 10px 12px;
    border-width: 1px;
    border-color: var(--border-soft);
    /* max-height намеренно НЕ выставлен: syngui не поддерживает
     * overflow: scroll внутри DecoratedBox, поэтому ограничение высоты
     * приводит к overflow содержимого за границы фона. Пусть карточка
     * растёт по контенту, а скролл всей ленты обрабатывается ScrollView. */
}

/* Тело tool-result. MarkdownView внутри: привязываем `--md-*` к теме точно
 * так же, как это делает `.msg-bubble-md` для ассистент-ответа — иначе
 * заголовки/HR/блок-цитаты рендерятся с дефолтными хардкод-цветами
 * (`#1E293B` heading на тёмной теме = невидимый, `#E5E7EB` hr =
 * «прострел» через текст заголовка).
 *
 * Базовый текст оставляем читаемым (без monospace по умолчанию): код
 * внутри fenced-блоков всё равно красится через `--md-code-block-*`,
 * а bash/web stdout без блоков смотрится как обычный параграф. */
.tool-result-body {
    font-size: 13px;
    color: var(--text-muted);
    --md-heading-color: var(--text);
    --md-link-color: var(--primary);
    --md-code-bg: var(--surface-hover);
    --md-code-color: var(--primary);
    --md-code-block-bg: var(--bg-window);
    --md-code-block-color: var(--text);
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

.tool-result-body-error {
    color: var(--error);
}

/* ───────────────────────── Confirm-overlay ─────────────────────────── */

.tool-confirm-empty {
    /* Pending=None: пустая заглушка; Portal всё равно закрыт. */
    background-color: transparent;
    width: 0;
    height: 0;
}

.tool-confirm-card {
    background-color: var(--bg-shell);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border);
    padding: 22px 24px;
    width: 520px;
    /* Portal передаёт детям containing_block = viewport, поэтому 80% — это
     * 80% высоты окна. Ниже шапки/предупреждения/кнопок остаётся скролл-блок
     * с аргументами; см. `.tool-confirm-args-wrap`. */
    max-height: 80%;
    box-shadow: var(--shadow-shell);
    animation: bubble-pop-in var(--duration-med) var(--ease-standard);
}

.tool-confirm-icon-wrap {
    background-color: var(--primary-soft);
    border-radius: 12px;
    width: 40px;
    height: 40px;
}

.tool-confirm-icon {
    color: var(--primary);
    font-size: 22px;
}

.tool-confirm-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
}

.tool-confirm-subtitle {
    font-size: 12px;
    color: var(--text-muted);
}

.tool-confirm-args-wrap {
    background-color: var(--bg-search);
    border-radius: 10px;
    padding: 12px 14px;
    border-width: 1px;
    border-color: var(--border-soft);
    min-height: 80px;
    transition: border-color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}

.tool-confirm-args-wrap:hover {
    border-color: var(--border);
}

/* ScrollView внутри args-wrap: корректно клиппит длинный код и HTML.
 *
 * Поведение по высоте:
 * - короткий контент (несколько строк) — ScrollView берёт натуральную высоту
 *   текста, карточка получается компактной;
 * - длинный — упирается в `max-height: 60%` viewport'а и включает скролл.
 *   Containing block в Portal — viewport, поэтому % считается от высоты окна.
 *   80% carda минус header/warning/buttons (~20%) → 60% — безопасное число,
 *   при котором содержимое всегда влазит в карточку без вертикального overflow.
 *
 * `max-height` ставим именно на ScrollView (не на DecoratedBox-обёртку
 * `.tool-confirm-args-wrap`), потому что ScrollView нативно клиппит контент
 * по своим bounds — без этого max-height на простом DecoratedBox оставлял
 * текст торчать за фоном (см. комментарий у `.tool-result-body-wrap`). */
.tool-confirm-args-scroll {
    scrollbar-width: 8px;
    max-height: 60%;
}

.tool-confirm-args-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-subtle);
    letter-spacing: 0.6px;
}

.tool-confirm-args {
    font-family: "JetBrains Mono", "Fira Code", monospace;
    font-size: 12px;
    color: var(--text);
    white-space: pre;
}

.tool-confirm-warning {
    font-size: 12px;
    color: var(--text-muted);
}

.tool-confirm-btn {
    border-radius: 10px;
    padding: 10px 14px;
    font-size: 13px;
    font-weight: 500;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard);
}

.tool-confirm-btn-secondary {
    background-color: var(--surface-hover);
    color: var(--text);
    border-width: 1px;
    border-color: var(--border);
}

.tool-confirm-btn-secondary:hover {
    background-color: var(--bg-search);
    border-color: var(--border-strong);
}

.tool-confirm-btn-primary {
    background-color: var(--primary);
    color: var(--on-primary);
    border-width: 1px;
    border-color: var(--primary);
}

.tool-confirm-btn-primary:hover {
    background-color: var(--primary-hover);
    border-color: var(--primary-hover);
}

.tool-confirm-btn-accent {
    background-color: transparent;
    color: var(--primary);
    border-width: 1px;
    border-color: var(--primary);
}

.tool-confirm-btn-accent:hover {
    background-color: var(--primary-soft);
}

/* ─────────────────────── Секция «Инструменты» ──────────────────────── */

.tools-section {
    background-color: var(--bg-search);
    border-radius: var(--radius-panel);
    padding: 14px 16px;
    border-width: 1px;
    border-color: var(--border-soft);
    transition: border-color var(--duration-med) var(--ease-standard);
}

.tools-section:hover {
    border-color: var(--border);
}

.tools-section-icon {
    color: var(--primary);
    font-size: 18px;
}

.tools-section-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
}

.tools-section-hint {
    font-size: 12px;
    color: var(--text-muted);
}

.tools-section-subtitle {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-subtle);
    letter-spacing: 0.6px;
}

/* Активный чип: primary-фон, белый текст и иконка — identичен визуалу
 * `.models-active-chip` (тот же стилевой язык во всём приложении). */
.tools-active-chip {
    background-color: var(--primary);
    color: var(--on-primary);
    border-radius: 16px;
    height: 32px;
    padding-left: 14px;
    padding-right: 14px;
    font-size: 12px;
    font-weight: 600;
    icon-size: 16px;
    accent-color: var(--on-primary);
    transition: background-color var(--duration-fast) var(--ease-standard),
                transform var(--duration-fast) var(--ease-standard);
}

.tools-active-chip:hover {
    background-color: var(--primary-hover);
    transform: translateY(-1px);
}

/* Доступный чип: outline-стиль с лёгкой заливкой на hover. */
.tools-available-chip {
    background-color: var(--bg-search);
    border-width: 1px;
    border-color: var(--border-strong);
    color: var(--text);
    border-radius: 18px;
    height: 34px;
    padding-left: 12px;
    padding-right: 14px;
    font-size: 12px;
    font-weight: 500;
    icon-size: 14px;
    accent-color: var(--primary);
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color   var(--duration-fast) var(--ease-standard),
                color          var(--duration-fast) var(--ease-standard),
                transform      var(--duration-fast) var(--ease-standard),
                box-shadow     var(--duration-fast) var(--ease-standard);
}

.tools-available-chip:hover {
    background-color: var(--primary-soft);
    border-color: var(--primary);
    color: var(--primary);
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(238, 94, 72, 0.16);
}

.tools-empty {
    background-color: var(--bg-panel);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 10px 12px;
}

.tools-empty-text {
    font-size: 12px;
    color: var(--text-subtle);
}
