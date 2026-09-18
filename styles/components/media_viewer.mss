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

.media-viewer-panzoom {
    background-color: transparent;
    width: 1100px;
    height: 680px;
}

.media-viewer-image {
    width: 1100px;
    height: 680px;
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

/* «Во весь экран»: карточка на всё окно, только кадр и панель плеера. */
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
