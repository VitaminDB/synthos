/* Полоса превью прикреплённых файлов над input-bar.
 * Появляется только при наличии хотя бы одного файла; пустой вариант
 * возвращает zero-size DecoratedBox, MSS-класс ниже даёт ему 0×0. */

.attachments-strip-empty {
    width: 0;
    height: 0;
    padding: 0 0 0 0;
    background-color: transparent;
    border-width: 0;
}

.attachments-strip-wrap {
    padding: 4px 0 8px 0;
    background-color: transparent;
    transition: opacity var(--duration-medium) var(--ease-standard),
                transform var(--duration-medium) var(--ease-standard);
}

.attachments-strip-scroll {
    /* ScrollView сам управляет своей высотой по контенту; нам важно лишь
     * не задавать ему фон, чтобы он сливался с input-panel-wrap. */
    background-color: transparent;
    border-width: 0;
    height: 132px;
}

/* Карточка thumbnail. 120×120 с скруглением и shadow на hover —
 * Material-эстетика. Overflow: hidden обрезает Image::Cover в скруглённый
 * квадрат. */
.attachment-card {
    width: 120px;
    height: 120px;
    border-radius: 12px;
    background-color: var(--surface);
    border-width: 1px;
    border-color: var(--border);
    overflow: hidden;
    transition: transform var(--duration-fast) var(--ease-standard),
                box-shadow var(--duration-fast) var(--ease-standard),
                border-color var(--duration-fast) var(--ease-standard);
}

.attachment-card:hover {
    transform: translateY(-2px);
    border-color: var(--primary);
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.18);
}

/* Картинка занимает всю карточку; Cover масштабирует с обрезкой по центру. */
.attachment-thumb {
    width: 120px;
    height: 120px;
}

/* Прозрачный overlay поверх Image — служит контейнером для close-крестика
 * в верхнем правом углу. Без явного фона, чтобы не закрывать картинку. */
.attachment-card-overlay {
    width: 120px;
    height: 120px;
    background-color: transparent;
    border-width: 0;
    padding: 4px 4px 0 0;
}

/* Close-кнопка: тёмный полупрозрачный круг, чтобы читалась на любой
 * картинке. Иконка контрастно-белая. Hover — увеличивает контраст. */
.attachment-card-close {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background-color: rgba(0, 0, 0, 0.55);
    color: #ffffff;
    transition: background-color var(--duration-fast) var(--ease-standard),
                transform var(--duration-fast) var(--ease-standard);
}

.attachment-card-close:hover {
    background-color: rgba(0, 0, 0, 0.78);
    transform: scale(1.08);
}
