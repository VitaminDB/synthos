/* Sortformer-нода (NodeKind::SortformerDiarizer) — Audio in → Text(JSON) out.
 *
 * Body использует общие .node-card-* / .node-input-* классы (как Demo).
 * Здесь только модификаторы pulse-анимации во время инференса и
 * Speaker-color легенда для будущего overlay внутри карточки (фаза 3). */

.sortformer-node-running {
    animation: sortformer-pulse 1100ms ease-in-out infinite alternate;
}

@keyframes sortformer-pulse {
    0%   { opacity: 0.60; }
    100% { opacity: 1.00; }
}

/* Speaker-цвета. Используются позже для timeline-overlay внутри карточки. */
.sortformer-speaker-0 { color: #4FC3F7; }
.sortformer-speaker-1 { color: #81C784; }
.sortformer-speaker-2 { color: #FFB74D; }
.sortformer-speaker-3 { color: #E57373; }
