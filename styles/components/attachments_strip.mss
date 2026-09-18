/* Карточки вложений: полоса над input-bar и плитка внутри пузырька.
 * Обе поверхности используют одну карточку `.attachment-card`, отличаясь
 * модификатором `.attachment-card-sent` (крупнее, без крестика).
 *
 * Пустой вариант возвращает zero-size DecoratedBox, чтобы Column не держал
 * вертикальный отступ под несуществующей полосой. */

.attachments-strip-empty {
    width: 0;
    height: 0;
    padding: 0 0 0 0;
    background-color: transparent;
    border-width: 0;
}

.attachments-strip-wrap {
    padding: 2px 0 6px 0;
    background-color: transparent;
    transition: opacity var(--duration-med) var(--ease-standard),
                transform var(--duration-med) var(--ease-standard);
}

.attachments-strip-scroll {
    /* Высота = карточка + место под горизонтальный скроллбар. */
    background-color: transparent;
    border-width: 0;
    height: 140px;
}

/* ── Карточка ───────────────────────────────────────────────────────────── */

.attachment-card {
    width: 124px;
    height: 124px;
    border-radius: 12px;
    background-color: var(--surface-hover);
    border-width: 1px;
    border-color: var(--border);
    overflow: hidden;
    cursor: pointer;
    transition: transform var(--duration-fast) var(--ease-standard),
                box-shadow var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard);
}

.attachment-card:hover {
    transform: translateY(-2px);
    border-color: var(--primary);
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.18);
}

/* В ленте карточка крупнее: пузырёк шире полосы ввода, и превью там —
 * основное содержимое сообщения, а не служебная плашка. */
.attachment-card-sent {
    width: 168px;
    height: 168px;
}

/* Картинка занимает всю карточку; Cover масштабирует с обрезкой по центру. */
.attachment-thumb {
    width: 124px;
    height: 124px;
}

.attachment-card-sent .attachment-thumb {
    width: 168px;
    height: 168px;
}

/* Заглушка для документов/аудио/прочих файлов — крупная иконка на surface. */
.attachment-icon-wrap {
    width: 124px;
    height: 124px;
    background-color: var(--surface-hover);
    border-width: 0;
}

.attachment-icon {
    icon-size: 40px;
    color: var(--text-subtle);
}

/* Плашка «готовим…» на время хеширования и ffmpeg-прогонов. */
.attachment-card-busy {
    border-color: var(--border-soft);
    background-color: var(--bg-search);
    cursor: default;
}

.attachment-card-busy:hover {
    transform: translateY(0);
    border-color: var(--border-soft);
    box-shadow: none;
}

/* ── Подпись ────────────────────────────────────────────────────────────── */

/* Прозрачный слой на всю карточку: прижимает подпись к нижней кромке. */
.attachment-card-caption-wrap {
    background-color: transparent;
    border-width: 0;
    padding: 0 0 0 0;
}

/* Тёмная плашка под именем — читается поверх любой картинки.
 *
 * Альфа высокая намеренно: композитор смешивает цвета в линейном
 * пространстве, поэтому «на глаз» полупрозрачная плашка получается
 * заметно светлее, чем то же значение в CSS (0.68 над #F3F3F5 даёт не
 * #565658, а #939394). 0.92 — то, что реально читается как scrim. */
.attachment-card-caption {
    background-color: rgba(10, 12, 16, 0.92);
    padding: 5px 8px 6px 8px;
    border-width: 0;
}

.attachment-card-name {
    color: #FFFFFF;
    font-size: 11px;
    font-weight: 500;
    line-clamp: 1;
}

.attachment-card-meta {
    color: rgba(255, 255, 255, 0.72);
    font-size: 10px;
}

/* ── Бейдж длительности ─────────────────────────────────────────────────── */

.attachment-card-badge-wrap {
    background-color: transparent;
    border-width: 0;
    padding: 6px 0 0 6px;
}

.attachment-card-badge {
    background-color: rgba(10, 12, 16, 0.9);
    border-radius: var(--radius-pill);
    padding: 2px 7px 2px 5px;
    border-width: 0;
}

.attachment-card-badge-icon {
    icon-size: 12px;
    color: #FFFFFF;
}

.attachment-card-badge-text {
    color: #FFFFFF;
    font-size: 10px;
    font-weight: 500;
}

/* ── Крестик удаления ───────────────────────────────────────────────────── */

/* Прозрачный overlay поверх превью — контейнер для крестика в углу.
 * Без явного фона, чтобы не закрывать картинку. */
.attachment-card-overlay {
    width: 124px;
    height: 124px;
    background-color: transparent;
    border-width: 0;
    padding: 4px 4px 0 0;
}

.attachment-card-close {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background-color: rgba(0, 0, 0, 0.82);
    color: #ffffff;
    icon-size: 14px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                transform var(--duration-fast) var(--ease-standard);
}

.attachment-card-close:hover {
    background-color: rgba(0, 0, 0, 0.94);
    transform: scale(1.08);
}

/* ── Плитка в пузырьке сообщения ────────────────────────────────────────── */

.msg-attachments {
    background-color: transparent;
    border-width: 0;
    padding: 0 0 2px 0;
}

/* ── Кнопка-скрепка в панели ввода ──────────────────────────────────────── */

.input-attach-wrap {
    background-color: transparent;
    border-width: 0;
}

ToolButton.input-attach {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background-color: transparent;
    color: var(--text-muted);
    icon-size: 19px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

ToolButton.input-attach:hover {
    background-color: var(--primary-soft);
    color: var(--primary);
}

/* Смайлик рядом со скрепкой — тот же вид (`ToolButton.input-attach`). */
.chat-emoji-picker {
    background-color: var(--bg-panel);
}

/* Подпись под полосой: количество файлов, суммарный вес и оценка того,
 * сколько контекста они займут. */
.attachments-strip-summary {
    color: var(--text-subtle);
    font-size: 11px;
    padding-left: 2px;
}

/* Галочка «передать модели путь к файлу» справа от подписи. Цвета коробки —
 * из глобального `Checkbox` (base/reset.mss), здесь только подпись в тон
 * соседней строке. */
Checkbox.attachments-share-path {
    color: var(--text-muted);
    font-size: 12px;
}

/* ── Инлайн-медиа в ленте (media_inline) ────────────────────────────────── */
/* Результат прогона пайплайна приходит вложением, и смотреть его через
 * модальное окно — лишний клик: карточка играет видео/аудио на месте и даёт
 * «Сохранить». Ширина ограничена, чтобы пузырёк не разъезжался на весь чат. */

.chat-media-card {
    width: 420px;
    border-radius: 12px;
    background-color: var(--surface-hover);
    border-width: 1px;
    border-color: var(--border);
    overflow: hidden;
}

.chat-media-stage {
    height: 236px;
    background-color: var(--bg-search);
    overflow: hidden;
    cursor: pointer;
}

/* Играющее видео: плеер (`components::video_player`, компактный) на месте
 * постера — та же высота, панель управления поверх кадра. Высота задана явно:
 * в ленте (VirtualList) высота сверху не ограничена, и без неё блок
 * получил бы запасные 100 px. */
.chat-media-video {
    height: 236px;
    background-color: #000000;
}

.chat-media-poster {
    width: 420px;
    height: 236px;
}

.chat-media-poster-empty {
    width: 420px;
    height: 236px;
    background-color: var(--bg-search);
}

.chat-media-image {
    width: 420px;
    height: 236px;
}

/* Кнопка ⏵ поверх постера: до клика видео не декодируется. */
.chat-media-play {
    width: 56px;
    height: 56px;
    border-radius: 28px;
    background-color: rgba(0, 0, 0, 0.55);
    color: var(--on-primary);
    font-size: 28px;
}

.chat-media-play:hover {
    background-color: var(--primary);
}

.chat-media-audio {
    padding: 10px 12px 10px 12px;
    background-color: var(--bg-search);
}

.chat-media-waveform {
    height: 64px;
}

.chat-media-play-small {
    width: 32px;
    height: 32px;
    border-radius: 16px;
    background-color: var(--surface-hover);
}

.chat-media-play-small:hover {
    background-color: var(--primary);
    color: var(--on-primary);
}

.chat-media-actions {
    padding: 6px 8px 6px 10px;
    background-color: var(--bg-panel);
    border-width: 0;
}

.chat-media-name {
    font-size: 12px;
    color: var(--text);
}

.chat-media-meta {
    font-size: 11px;
    color: var(--text-muted);
}

.chat-media-btn {
    width: 28px;
    height: 28px;
    border-radius: 8px;
}

.chat-media-btn:hover {
    background-color: var(--surface-hover);
}
