.input-panel-wrap {
    padding: 0 24px 20px 24px;
    background-color: var(--bg-chat);
}

.input-panel {
    background-color: var(--bg-shell);
    border-width: 1px;
    border-color: var(--border);
    border-radius: var(--radius-panel);
    padding: 14px 16px 14px 16px;
    transition: border-color var(--duration-fast) var(--ease-standard),
                box-shadow   var(--duration-fast) var(--ease-standard);
}

.input-panel:focus {
    border-color: var(--primary);
    box-shadow: 0 0 0 3px rgba(238, 94, 72, 0.18);
}

.input-pen       { icon-size: 20px; color: var(--text-muted); }
.input-sparkle   { icon-size: 20px; color: var(--text-muted); }
.input-placeholder { color: var(--text-subtle); font-size: 14px; }

.input-divider {
    height: 1px;
    background-color: var(--border-soft);
}

.input-tool {
    border-radius: 8px;
    background-color: transparent;
    color: var(--text-muted);
    transition: background-color var(--duration-fast) var(--ease-standard),
                color            var(--duration-fast) var(--ease-standard);
}

.input-tool:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.input-send {
    border-radius: 50%;
    background-color: var(--primary);
    color: var(--on-primary);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.input-send:hover { background-color: var(--primary-hover); }

/* Чип-счётчик токенов ввода: "⚡ N ток." слева от кнопки отправки.
 * Мягкий surface + тонкая рамка; число/иконка — мутовые по умолчанию,
 * transition чтобы плавно появлялся при первом нажатии символа. */
.input-token-chip {
    background-color: var(--surface-hover);
    border-width: 1px;
    border-color: var(--border);
    border-radius: 999px;
    padding: 6px 12px 6px 10px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color     var(--duration-fast) var(--ease-standard),
                color            var(--duration-fast) var(--ease-standard);
}

.input-token-chip:hover {
    background-color: var(--surface-active, var(--surface-hover));
    border-color: var(--primary);
}

.input-token-chip-icon {
    icon-size: 18px;
    color: var(--primary);
}

.input-token-chip-text {
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
}

/* Заглушка нулевого размера — когда ввод пуст, чип не показывается,
 * чтобы не «светить» нулём и не держать пустое место в правой группе. */
.input-token-chip-empty {
    width: 0;
    height: 0;
    background-color: transparent;
    border-width: 0;
    padding: 0 0 0 0;
}

/* ─────────────── Кнопка regenerate в input-panel ──────────────── */

/* Обёртка нужна как «hook» для DecoratedBox-обёртки; собственных стилей
 * не имеет — все размеры берёт ToolButton ниже. */
.input-regen-wrap {
    background-color: transparent;
}

ToolButton.input-regen {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background-color: var(--surface-hover);
    color: var(--text-muted);
    icon-size: 18px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color            var(--duration-fast) var(--ease-standard),
                transform        var(--duration-fast) var(--ease-standard);
}

ToolButton.input-regen:hover {
    background-color: var(--primary-soft);
    color: var(--primary);
    transform: rotate(-30deg);
}

ToolButton.input-regen:active {
    transform: rotate(-90deg);
}

/* Скрытое состояние — когда нечего регенерировать (пустая лента или
 * pending=true). Zero-size без фона/рамки, чтобы Row не оставлял
 * визуальной «дыры» рядом с кнопкой отправки. */
.input-regen-empty {
    width: 0;
    height: 0;
    background-color: transparent;
}

/* Вариант кнопки во время активного стрима: красный акцент = «Остановить». */
.input-send-stop {
    background-color: var(--error);
    color: var(--on-primary);
}

.input-send-stop:hover {
    background-color: var(--error-hover);
}

/* Обёртка `DecoratedBox` вокруг `MultilineTextEdit` — снимает фон и рамку,
 * чтобы поле выглядело «встроенным» в общую `.input-panel`. */
.chat-input-field {
    background-color: transparent;
    border-width: 0;
    color: var(--text);
    font-size: 14px;
    padding: 0 0 0 0;
}

/* Сам MultilineTextEdit: фон/текст/рамка из переменных темы, курсор и
 * focus-рамка — из `accent-color`, hover-рамка анимируется до `--primary`. */
.chat-input-edit {
    background-color: var(--bg-shell);
    color: var(--text);
    border-color: var(--border);
    accent-color: var(--primary);
    transition: border-color var(--duration-fast) var(--ease-standard);
}

.chat-input-edit:hover {
    border-color: var(--primary);
}

/* ── Микрофон / waveform ─────────────────────────────────────────────────── */

/* Активная запись: иконка MI_STOP, цвет акцентный, мягкое мерцание. */
.input-mic-recording {
    color: var(--error);
    background-color: rgba(229, 57, 53, 0.12);
    border-radius: 8px;
    animation: input-mic-pulse 1.2s ease-in-out infinite;
}

/* Идёт распознавание (transcribing). Иконка часов + статичный фон, без pulse. */
.input-mic-busy {
    color: var(--primary);
    background-color: rgba(59, 130, 246, 0.10);
    border-radius: 8px;
}

@keyframes input-mic-pulse {
    0%   { opacity: 1.0; }
    50%  { opacity: 0.55; }
    100% { opacity: 1.0; }
}

/* Контейнер waveform-визуализации — появляется между редактором и тулбаром.
 * `transition: background-color` сглаживает появление; высоту задаёт сам
 * Canvas (фиксированная), а в idle-состоянии заменяется на пустой контейнер. */
.input-waveform-wrap {
    background-color: var(--bg-chat);
    border-radius: 10px;
    padding: 4px 12px 4px 12px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

/* Idle: waveform-row невидима. height: 0 → не съедает вертикальный gap. */
.input-waveform-empty {
    height: 0;
    padding: 0 0 0 0;
    margin: 0 0 0 0;
}

/* Сам Canvas. accent-color → цвет fill в RMS-bars. */
.input-waveform {
    accent-color: var(--primary);
}
