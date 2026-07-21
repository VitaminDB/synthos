/* ASR-нода (NodeKind::AsrGigaam) — Audio in → Text out.
 *
 * Body использует стандартные node-card-* и node-input-* классы (как у
 * Demo-ноды через field_row). Здесь только модификатор pulse-анимации
 * на Play-кнопке при работе и небольшой акцент для filename.
 */

.asr-node-running {
    /* Чуть приглушённый акцент во время инференса — отличает от idle Play. */
    animation: asr-pulse 1100ms ease-in-out infinite alternate;
}

@keyframes asr-pulse {
    0%   { opacity: 0.60; }
    100% { opacity: 1.00; }
}

/* TextView нода: editable multiline text. Использует .node-input-text
 * (унаследовано с Demo-ноды), но добавляем большую min-height для удобства
 * редактирования транскрибата. */
.text-view-host {
    background-color: transparent;
    padding: 4px 10px 8px 10px;
}

.text-view-editor {
    min-height: 64px;
}
