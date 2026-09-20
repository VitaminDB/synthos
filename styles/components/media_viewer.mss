/* Просмотрщик вложений (Portal поверх страницы чата).
 *
 * Окно — в теме приложения: панель, текст, рамки и кнопки на её токенах.
 * Тёмными остаются только сцены, где тема ни при чём: кадр видео и
 * затемнение поверх размытого фона картинки.
 *
 * Фильтров GPU (`filter`, `backdrop-filter`) здесь нет намеренно: приложение
 * рисуется встроенной графикой, и полноэкранные проходы размытия на каждый
 * кадр масштабирования подвешивали интерфейс. */

.media-viewer {
    width: 1280px;
    height: 860px;
    max-width: 96%;
    max-height: 94%;
    background-color: var(--bg-panel);
    color: var(--text);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border-strong);
    box-shadow: 0 32px 80px rgba(0, 0, 0, 0.45);
    overflow: hidden;
}

.media-viewer-empty {
    width: 0;
    height: 0;
    background-color: transparent;
    border-width: 0;
    padding: 0 0 0 0;
}

/* ── Шапка ──────────────────────────────────────────────────────────────── */

.media-viewer-header {
    background-color: var(--bg-panel);
    padding: 8px 14px 8px 16px;
    border-width: 0;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.media-viewer-kind-icon {
    icon-size: 22px;
    color: var(--text-muted);
}

.media-viewer-title {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
    line-clamp: 1;
}

.media-viewer-subtitle {
    color: var(--text-muted);
    font-size: 12px;
}

ToolButton.media-viewer-action {
    width: 32px;
    height: 32px;
    border-radius: 8px;
    background-color: transparent;
    color: var(--text-muted);
    icon-size: 18px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

ToolButton.media-viewer-action:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

ToolButton.media-viewer-close:hover {
    background-color: var(--error);
    color: #FFFFFF;
}

/* За неё окно тащат; высота задана, потому что ручка внутри занимает всё,
 * что ей дадут. */
.media-viewer-title-area {
    background-color: transparent;
    border-width: 0;
    padding: 0 0 0 0;
    height: 40px;
    cursor: move;
}

.media-viewer-action-sep {
    width: 1px;
    height: 18px;
    margin-left: 4px;
    margin-right: 4px;
    background-color: var(--border-strong);
    border-width: 0;
}

/* ── Тело ───────────────────────────────────────────────────────────────── */

.media-viewer-body {
    background-color: var(--bg-window);
    padding: 0 0 0 0;
    border-width: 0;
    flex-grow: 1;
}

/* Сцена растягивается между стрелками листания. */
.media-viewer-stage {
    background-color: transparent;
    border-width: 0;
    flex-grow: 1;
    height: 680px;
    overflow: hidden;
}

/* Сцена картинки — от края до края тела, любого размера: окно тянут и
 * разворачивают. Что внутри — см. «Сцена картинки» ниже. */
.media-viewer-image-stage {
    background-color: var(--bg-window);
    border-width: 0;
    padding: 0 0 0 0;
    flex-grow: 1;
    overflow: hidden;
}

/* Видео: сцена без фиксированной высоты — плеер (`components::video_player`,
 * стили в video_player.mss) занимает всё тело между шапкой и низом карточки;
 * подвала у видео нет. */
.media-viewer-video-stage {
    background-color: #000000;
    border-width: 0;
    padding: 0 0 0 0;
    flex-grow: 1;
    overflow: hidden;
}

/* Развёрнуто на всё окно приложения: шапка остаётся, рамка и тень уходят. */
.media-viewer.media-viewer-max {
    width: 100%;
    height: 100%;
    max-width: 100%;
    max-height: 100%;
    border-radius: 0;
    border-width: 0;
    box-shadow: none;
}

/* «Во весь экран»: карточка на всё окно, только сцена и её панель. */
.media-viewer.media-viewer-full {
    width: 100%;
    height: 100%;
    max-width: 100%;
    max-height: 100%;
    background-color: #000000;
    border-radius: 0;
    border-width: 0;
    box-shadow: none;
}

/* ── Аудио ──────────────────────────────────────────────────────────────── */

.media-viewer-audio {
    background-color: transparent;
    border-width: 0;
    padding: 40px 60px 40px 60px;
    width: 1000px;
    height: 660px;
}

.media-viewer-audio-name {
    color: var(--text);
    font-size: 16px;
    font-weight: 600;
    text-align: center;
}

.media-viewer-audio-meta {
    color: var(--text-muted);
    font-size: 12px;
    text-align: center;
}

.media-viewer-waveform {
    accent-color: var(--primary);
    color: var(--text-subtle);
    background-color: var(--bg-search);
    border-radius: 12px;
}

ToolButton.media-viewer-play {
    width: 52px;
    height: 52px;
    border-radius: 50%;
    background-color: var(--primary);
    color: var(--on-primary);
    icon-size: 26px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                transform var(--duration-fast) var(--ease-standard);
}

ToolButton.media-viewer-play:hover {
    background-color: var(--primary-hover);
    transform: scale(1.05);
}

/* ── Документ ───────────────────────────────────────────────────────────── */

.media-viewer-doc {
    background-color: var(--bg-window);
    border-width: 0;
    padding: 0 0 0 0;
    width: 1080px;
    height: 660px;
}

.media-viewer-doc-scroll {
    background-color: transparent;
    border-width: 0;
    height: 660px;
}

.media-viewer-doc-text {
    color: var(--text);
    font-family: monospace;
    font-size: 13px;
    line-height: 20px;
    padding: 18px 22px 18px 22px;
}

/* ── Пустые состояния и подсказки ───────────────────────────────────────── */

.media-viewer-big-icon {
    icon-size: 72px;
    color: var(--text-subtle);
}

.media-viewer-hint {
    color: var(--text-muted);
    font-size: 13px;
    text-align: center;
}

/* ── Листание ───────────────────────────────────────────────────────────── */

ToolButton.media-viewer-nav {
    width: 46px;
    height: 46px;
    border-radius: 50%;
    background-color: var(--bg-panel);
    color: var(--text-muted);
    icon-size: 26px;
    margin-left: 10px;
    margin-right: 10px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                transform var(--duration-fast) var(--ease-standard);
}

ToolButton.media-viewer-nav:hover {
    background-color: var(--surface-hover);
    color: var(--text);
    transform: scale(1.06);
}

/* Распорка вместо стрелок, когда вложение одно: сцена не должна съезжать. */
.media-viewer-nav-spacer {
    width: 16px;
    height: 1px;
    background-color: transparent;
    border-width: 0;
}

/* ── Подвал ─────────────────────────────────────────────────────────────── */

.media-viewer-footer {
    background-color: var(--bg-panel);
    padding: 8px 14px 8px 14px;
    border-width: 0;
    border-top-width: 1px;
    border-color: var(--border-soft);
}

.media-viewer-zoom {
    color: var(--text-muted);
    font-size: 12px;
    width: 48px;
    text-align: center;
}

.media-viewer-counter {
    color: var(--text-muted);
    font-size: 12px;
}

/* ── Ручки ресайза ──────────────────────────────────────────────────────────
 * Полоски в 4 px по краям карточки и уголки 14 px. Тоньше полос прокрутки
 * сцены (те отстоят от края на 4 px), чтобы не перекрывать их. */

.media-viewer-grip {
    background-color: transparent;
    border-width: 0;
    padding: 0 0 0 0;
}

.media-viewer-grip-fill {
    background-color: transparent;
    border-width: 0;
    width: 100%;
    height: 100%;
}

.media-viewer-grip-x {
    width: 4px;
    cursor: ew-resize;
}

.media-viewer-grip-y {
    height: 4px;
    cursor: ns-resize;
}

.media-viewer-grip-se {
    width: 14px;
    height: 14px;
    cursor: nwse-resize;
}

.media-viewer-grip-sw {
    width: 14px;
    height: 14px;
    cursor: nesw-resize;
}

/* ── Сцена картинки (`pages::syn_chat::image_stage`) ───────────────────────
 * Слои: размытая копия картинки → затемнение → область просмотра → стрелки,
 * лента миниатюр и панель инструментов. Панели — цвета темы, непрозрачные. */

.iv-stage {
    background-color: var(--bg-window);
    border-width: 0;
    padding: 0 0 0 0;
    width: 100%;
    height: 100%;
    overflow: hidden;
}

/* Фон вместо пустых полей: заранее размытая крошечная копия картинки
 * (`<sha>.blur.png`), растянутая на сцену. Пока её нет — цвет сцены. */
.iv-backdrop {
    width: 100%;
    height: 100%;
}

.iv-backdrop-empty {
    background-color: transparent;
    border-width: 0;
    width: 100%;
    height: 100%;
}

/* Затемнение поверх размытой копии: картинка должна оставаться главной.
 * Внутренняя тень — виньетка к краям сцены. */
.iv-scrim {
    background: linear-gradient(to bottom, rgba(8, 9, 12, 0.42), rgba(8, 9, 12, 0.55) 60%, rgba(8, 9, 12, 0.74));
    box-shadow: inset 0 0 120px rgba(0, 0, 0, 0.45);
    border-width: 0;
    width: 100%;
    height: 100%;
}

/* `color` — цвет ползунков полос прокрутки и подписи «Загрузка…»: они лежат
 * на затемнённом фоне, поэтому белые в любой теме. */
.iv-viewport {
    background-color: transparent;
    color: #FFFFFF;
}

ToolButton.iv-nav {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border-strong);
    color: var(--text-muted);
    icon-size: 26px;
    margin-left: 16px;
    margin-right: 16px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                transform var(--duration-fast) var(--ease-standard);
}

ToolButton.iv-nav:hover {
    background-color: var(--surface-hover);
    color: var(--text);
    transform: scale(1.06);
}

/* Панель инструментов: пилюля внизу по центру. */
.iv-toolbar {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border-strong);
    border-radius: 16px;
    padding: 5px 8px 5px 8px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.40);
}

.iv-bottom-gap {
    width: 1px;
    height: 6px;
    background-color: transparent;
    border-width: 0;
}

ToolButton.iv-tool {
    width: 34px;
    height: 34px;
    border-radius: 10px;
    background-color: transparent;
    color: var(--text-muted);
    icon-size: 20px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

ToolButton.iv-tool:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

Button.iv-tool-text {
    height: 34px;
    min-width: 40px;
    padding: 0 8px 0 8px;
    border-radius: 10px;
    border-width: 0;
    background-color: transparent;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 700;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

Button.iv-tool-text:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

.iv-tool-sep {
    width: 1px;
    height: 18px;
    margin-left: 5px;
    margin-right: 5px;
    background-color: var(--border-strong);
    border-width: 0;
}

.iv-zoom {
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
    width: 52px;
    text-align: center;
}

.iv-counter {
    color: var(--text-muted);
    font-size: 12px;
    padding: 0 6px 0 6px;
}

/* Лента миниатюр над панелью. */
.iv-strip {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border-strong);
    border-radius: 14px;
    padding: 6px 6px 6px 6px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.40);
}

.iv-thumb {
    width: 52px;
    height: 52px;
    border-radius: 9px;
    border-width: 2px;
    border-color: transparent;
    background-color: var(--bg-search);
    overflow: hidden;
    opacity: 0.62;
    transition: opacity var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard);
}

.iv-thumb:hover {
    opacity: 1;
}

.iv-thumb-active {
    opacity: 1;
    border-color: var(--primary);
}

.iv-thumb-img {
    width: 100%;
    height: 100%;
}

.iv-thumb-icon {
    icon-size: 24px;
    color: var(--text-muted);
}
