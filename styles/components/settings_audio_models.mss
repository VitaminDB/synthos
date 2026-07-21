/* Стили подстраницы «Аудио модели» (`settings/audio_models`).
 *
 * Класс `.audio-models-page` добавляется к корневому DecoratedBox страницы
 * вместе с `.settings-page` и `.models-page` — это даёт всю стилистику
 * страницы текстовых моделей (cards, chips, paths) бесплатно. Здесь —
 * только специфика ASR-страницы: карточка управления процессом сервера. */

.audio-models-page {
    /* унаследовано от .settings-page; зарезервировано под будущие правки */
}

/* ── Карточка управления ASR-сервером ─────────────────────────────────── */

/* Тонкая акцентная линия слева — визуально отличает control-card от
 * соседних .settings-card. Не агрессивно: разный фон делает её живой,
 * но без отдельного «бэйджа». */
.audio-server-card {
    border-color: var(--primary-soft);
    background-color: var(--bg-shell);
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color     var(--duration-fast) var(--ease-standard);
}

.audio-server-card:hover {
    background-color: var(--bg-panel);
    border-color: var(--primary);
}
