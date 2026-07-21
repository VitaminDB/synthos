/* Окно распознавания (FAB → центр). Frosted-glass карточка через
 * `backdrop-filter: blur(...)` — фон за окном размывается, эффект «стеклянной
 * панели». Цвета берутся из тематических токенов `--glass-*`, которые
 * SynthosTheme::to_mss() задаёт под каждую тему (светлые и тёмные значения
 * с альфой). Без захардкоженного белого — на тёмной теме панель остаётся
 * затемнённым стеклом, на светлой — белёсым стеклом. */

.voice-overlay-card {
    background-color: var(--glass-card-bg);
    backdrop-filter: blur(28px);
    border-radius: 24px;
    padding: 36px 48px 28px 48px;
    min-width: 880px;
    min-height: 720px;
    box-shadow: var(--glass-shadow);
    border-width: 1px;
    border-color: var(--glass-border);
    animation: voice-panel-fade-in 320ms var(--ease-standard);
}

/* Stack аура+FAB размером 480×480 — фиксированный квадрат, чтобы канвас
 * не схлопывался в zero. Центр (240,240) совпадает с центром FAB-кнопки. */
.voice-aura-wrap {
    width: 480px;
    height: 480px;
}

.voice-aura {
    accent-color: var(--primary);
}

/* Статус-текст центрируется вместе с Column.cross_axis_alignment(Center). */
.voice-overlay-status {
    font-size: 14px;
    color: var(--text-muted);
}

/* Контейнер двух колонок (raw слева, refined справа) — прозрачный, нужен
 * только для горизонтального отступа от карточки. Сам Row делит ширину
 * между двумя `.grow`-колонками. */
.voice-overlay-transcripts {
    width: 780px;
    background-color: transparent;
}

/* Текст распознавания — через MSS-переменные, которые выставляются в
 * lib.rs::build_context effect'ом из Settings → Общие → шрифт. Фон и рамка
 * прозрачные: реальная glass-подложка с бордером лежит на воротничке
 * `.voice-overlay-text-raw` / `.voice-overlay-text-refined`. Без `border-color:
 * transparent` MultilineTextEdit рисует свою дефолтную светло-серую рамку
 * (`#D1D5DB`), которая в тёмной теме выглядит как «белый бордер». */
.voice-overlay-text {
    font-family: var(--voice-font-family);
    font-size: var(--voice-font-size);
    line-height: 1.4;
    color: var(--text);
    background-color: transparent;
    border-color: transparent;
    caret-color: var(--primary);
}

.voice-overlay-placeholder {
    font-family: var(--voice-font-family);
    font-size: var(--voice-font-size);
    color: var(--text-subtle);
    line-height: 1.4;
}

/* Заголовок секции — мелкий subtitle перед каждым textarea. */
.voice-overlay-section-title {
    font-size: 12px;
    color: var(--text-muted);
    letter-spacing: 0.4px;
}

/* Карточка вокруг raw-textarea: тематический glass-фон. max-height ограничивает
 * рост обёртки даже если MultilineTextEdit вычислил больше — страховка от
 * выезда за границы карточки на крупных стенограммах. */
.voice-overlay-text-raw {
    background-color: var(--glass-field-bg);
    border-radius: 12px;
    padding: 10px 12px 10px 12px;
    border-width: 1px;
    border-color: var(--glass-border);
    max-height: 140px;
}

/* Карточка вокруг refined-textarea: тот же фон + цветной accent-border слева,
 * чтобы пользователь видел «а вот результат модели». */
.voice-overlay-text-refined {
    background-color: var(--glass-field-bg);
    border-radius: 12px;
    padding: 10px 12px 10px 12px;
    border-width: 1px;
    border-color: var(--glass-border);
    border-left-width: 3px;
    border-left-color: var(--primary);
    max-height: 140px;
}

/* Центральная Refine-кнопка между двумя колонками. На акцентном цвете темы —
 * визуально явная, в отличие от старого мелкого ToolButton'а в шапке. */
.voice-action-refine {
    color: var(--on-primary);
    background-color: var(--primary);
    border-radius: 14px;
    padding: 10px;
}
.voice-action-refine:hover {
    background-color: var(--primary-hover);
}
.voice-action-refine:disabled {
    background-color: var(--surface-hover);
    color: var(--text-subtle);
}

/* Сообщение об ошибке постобработки. Mss `text-overflow: ellipsis` уже даёт
 * красивое усечение длинных URL'ов; цвет — error. */
.voice-overlay-refine-error {
    font-size: 12px;
    color: var(--error);
    line-height: 1.3;
}

.voice-actions-row {
    padding: 8px 0 0 0;
}

.voice-action-pause   { color: var(--primary); }
.voice-action-stop    { color: var(--error); }
.voice-action-resume  { color: var(--primary); }
.voice-action-copy,
.voice-action-paste,
.voice-action-restart {
    color: var(--text);
}
.voice-action-busy { color: var(--primary); }

.voice-action-pause:hover,
.voice-action-stop:hover,
.voice-action-resume:hover,
.voice-action-copy:hover,
.voice-action-paste:hover,
.voice-action-restart:hover {
    background-color: var(--surface-hover);
    border-radius: 8px;
}

@keyframes voice-panel-fade-in {
    from { opacity: 0; transform: scale(0.92) translateY(20px); }
    to   { opacity: 1; transform: scale(1.0)  translateY(0); }
}
