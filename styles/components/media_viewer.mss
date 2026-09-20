/* Полноэкранный просмотрщик вложений (Portal поверх страницы чата).
 *
 * Тёмная «фотогалерейная» палитра независимо от темы приложения: под
 * картинкой и видео нейтральный тёмный фон читается лучше светлого, а сам
 * просмотрщик — модальный слой, а не часть страницы. */

.media-viewer {
    width: 1280px;
    height: 860px;
    max-width: 96%;
    max-height: 94%;
    background-color: #14151A;
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: rgba(255, 255, 255, 0.10);
    box-shadow: 0 32px 80px rgba(0, 0, 0, 0.55);
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
    background-color: rgba(255, 255, 255, 0.04);
    padding: 12px 14px 12px 16px;
    border-width: 0;
    border-bottom-width: 1px;
    border-color: rgba(255, 255, 255, 0.08);
}

.media-viewer-kind-icon {
    icon-size: 22px;
    color: rgba(255, 255, 255, 0.62);
}

.media-viewer-title {
    color: #F2F3F5;
    font-size: 14px;
    font-weight: 600;
    line-clamp: 1;
}

.media-viewer-subtitle {
    color: rgba(255, 255, 255, 0.52);
    font-size: 12px;
}

ToolButton.media-viewer-action {
    width: 32px;
    height: 32px;
    border-radius: 8px;
    background-color: transparent;
    color: rgba(255, 255, 255, 0.72);
    icon-size: 18px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

ToolButton.media-viewer-action:hover {
    background-color: rgba(255, 255, 255, 0.12);
    color: #FFFFFF;
}

ToolButton.media-viewer-close:hover {
    background-color: var(--error);
    color: #FFFFFF;
}

.media-viewer-title-area {
    background-color: transparent;
    border-width: 0;
    padding: 0 0 0 0;
    flex-grow: 1;
}

.media-viewer-action-sep {
    width: 1px;
    height: 18px;
    margin-left: 4px;
    margin-right: 4px;
    background-color: rgba(255, 255, 255, 0.14);
    border-width: 0;
}

/* ── Тело ───────────────────────────────────────────────────────────────── */

.media-viewer-body {
    background-color: #0E0F13;
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
    background-color: #0B0C10;
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
    color: #F2F3F5;
    font-size: 16px;
    font-weight: 600;
    text-align: center;
}

.media-viewer-audio-meta {
    color: rgba(255, 255, 255, 0.52);
    font-size: 12px;
    text-align: center;
}

.media-viewer-waveform {
    accent-color: var(--primary);
    color: rgba(255, 255, 255, 0.28);
    background-color: rgba(255, 255, 255, 0.04);
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
    background-color: #16171C;
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
    color: #D7DAE0;
    font-family: monospace;
    font-size: 13px;
    line-height: 20px;
    padding: 18px 22px 18px 22px;
}

/* ── Пустые состояния и подсказки ───────────────────────────────────────── */

.media-viewer-big-icon {
    icon-size: 72px;
    color: rgba(255, 255, 255, 0.32);
}

.media-viewer-hint {
    color: rgba(255, 255, 255, 0.52);
    font-size: 13px;
    text-align: center;
}

/* ── Листание ───────────────────────────────────────────────────────────── */

ToolButton.media-viewer-nav {
    width: 46px;
    height: 46px;
    border-radius: 50%;
    background-color: rgba(255, 255, 255, 0.08);
    color: rgba(255, 255, 255, 0.82);
    icon-size: 26px;
    margin-left: 10px;
    margin-right: 10px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                transform var(--duration-fast) var(--ease-standard);
}

ToolButton.media-viewer-nav:hover {
    background-color: rgba(255, 255, 255, 0.18);
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
    background-color: rgba(255, 255, 255, 0.04);
    padding: 8px 14px 8px 14px;
    border-width: 0;
    border-top-width: 1px;
    border-color: rgba(255, 255, 255, 0.08);
}

.media-viewer-zoom {
    color: rgba(255, 255, 255, 0.72);
    font-size: 12px;
    width: 48px;
    text-align: center;
}

.media-viewer-counter {
    color: rgba(255, 255, 255, 0.52);
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
 * лента миниатюр и панель инструментов. */

.iv-stage {
    background-color: #0B0C10;
    border-width: 0;
    padding: 0 0 0 0;
    width: 100%;
    height: 100%;
    overflow: hidden;
}

/* Фон вместо чёрных полей. `scale` прячет за край сцены прозрачную кайму,
 * которую размытие даёт по периметру слоя. */
.iv-backdrop {
    background-color: transparent;
    border-width: 0;
    padding: 0 0 0 0;
    width: 100%;
    height: 100%;
    filter: blur(32px) blur(32px) saturate(1.3);
    scale: 1.18;
}

.iv-backdrop-img {
    width: 100%;
    height: 100%;
}

/* Затемнение поверх размытой копии: картинка должна оставаться главной, а
 * белые панели — читаться на любом фоне. */
.iv-scrim {
    background: linear-gradient(to bottom, rgba(8, 9, 12, 0.50), rgba(8, 9, 12, 0.62) 60%, rgba(8, 9, 12, 0.82));
    /* Размытие у края слоя смешивается с пустотой за ним и даёт тёмную
     * кайму; внутренняя тень превращает её в обычную виньетку. */
    box-shadow: inset 0 0 120px rgba(0, 0, 0, 0.55);
    border-width: 0;
    width: 100%;
    height: 100%;
}

/* `color` — цвет ползунков полос прокрутки и подписи «Загрузка…». */
.iv-viewport {
    background-color: transparent;
    color: #FFFFFF;
}

ToolButton.iv-nav {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    background-color: rgba(16, 17, 22, 0.55);
    color: rgba(255, 255, 255, 0.86);
    icon-size: 26px;
    margin-left: 16px;
    margin-right: 16px;
    backdrop-filter: blur(16px);
    transition: background-color var(--duration-fast) var(--ease-standard),
                transform var(--duration-fast) var(--ease-standard);
}

ToolButton.iv-nav:hover {
    background-color: rgba(40, 42, 52, 0.80);
    color: #FFFFFF;
    transform: scale(1.06);
}

/* Панель инструментов: «стеклянная» пилюля внизу по центру. */
.iv-toolbar {
    background-color: rgba(18, 19, 25, 0.62);
    backdrop-filter: blur(24px);
    border-width: 1px;
    border-color: rgba(255, 255, 255, 0.12);
    border-radius: 16px;
    padding: 5px 8px 5px 8px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
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
    color: rgba(255, 255, 255, 0.78);
    icon-size: 20px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

ToolButton.iv-tool:hover {
    background-color: rgba(255, 255, 255, 0.14);
    color: #FFFFFF;
}

Button.iv-tool-text {
    height: 34px;
    min-width: 40px;
    padding: 0 8px 0 8px;
    border-radius: 10px;
    border-width: 0;
    background-color: transparent;
    color: rgba(255, 255, 255, 0.78);
    font-size: 12px;
    font-weight: 700;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

Button.iv-tool-text:hover {
    background-color: rgba(255, 255, 255, 0.14);
    color: #FFFFFF;
}

.iv-tool-sep {
    width: 1px;
    height: 18px;
    margin-left: 5px;
    margin-right: 5px;
    background-color: rgba(255, 255, 255, 0.14);
    border-width: 0;
}

.iv-zoom {
    color: #FFFFFF;
    font-size: 12px;
    font-weight: 600;
    width: 52px;
    text-align: center;
}

.iv-counter {
    color: rgba(255, 255, 255, 0.72);
    font-size: 12px;
    padding: 0 6px 0 6px;
}

/* Лента миниатюр над панелью. */
.iv-strip {
    background-color: rgba(18, 19, 25, 0.55);
    backdrop-filter: blur(24px);
    border-width: 1px;
    border-color: rgba(255, 255, 255, 0.10);
    border-radius: 14px;
    padding: 6px 6px 6px 6px;
}

.iv-thumb {
    width: 52px;
    height: 52px;
    border-radius: 9px;
    border-width: 2px;
    border-color: transparent;
    background-color: rgba(255, 255, 255, 0.06);
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
    color: rgba(255, 255, 255, 0.62);
}
