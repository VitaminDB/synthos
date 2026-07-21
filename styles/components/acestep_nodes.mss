/* ACE-Step v1.5 семейство нод (8 шт): TextEncoder, LyricEncoder,
 * TimbreEncoder, Pack, VaeEncode, VaeDecode, ArLm, Sampler.
 *
 * Все используют один общий визуальный язык — фиолетово-розовая палитра
 * для отличия от других нейро-нод (OmniVoice indigo, GigaAM orchid,
 * Sortformer cyan). Размер карточки 380px (как у OmniVoice — у Sampler'а
 * больше всего полей, остальные подравниваются).
 */

.acestep-text-node,
.acestep-lyric-node,
.acestep-timbre-node,
.acestep-pack-node,
.acestep-vae-encode-node,
.acestep-vae-decode-node,
.acestep-ar-lm-node,
.acestep-sampler-node {
    width: 380px;
}

/* Pack — без model_path, всего 1 строка hint'а; делаем чуть уже. */
.acestep-pack-node {
    width: 320px;
}

/* Распределение [label 30% / control 70%] в field-row'ах. Cells получают
 * жёсткую пропорцию через flex-grow на DecoratedBox-обёртках; контрол
 * внутри control-cell наследует доступную ширину (см. acestep-slider-stretch).
 */
.acestep-field-row {
    width: auto;
}

.acestep-field-label-cell {
    flex-grow: 3;
    padding-left: 10px;
    padding-right: 10px;
    padding-top: 2px;
    padding-bottom: 2px;
}

.acestep-field-control-cell {
    flex-grow: 7;
    padding-left: 10px;
    padding-right: 10px;
    padding-top: 2px;
    padding-bottom: 2px;
}

/* Лейбл — текст без фиксированной ширины; растягивается label-cell'ом. */
.acestep-field-label {
    flex-grow: 0;
}

/* Slider в control-cell — растягиваем (вместо фиксированных 120px). */
.acestep-slider-stretch {
    width: auto;
    flex-grow: 1;
}

/* Pulse-анимация для статус-строки активной ноды (running=true). */
.acestep-node-running {
    animation: acestep-pulse 1100ms ease-in-out infinite alternate;
}

/* Подсказка про модель: какой .syn bundle нужен в данную ноду.
 * Чуть более акцентированный цвет чем у обычного node-card-hint
 * (фиолетово-розовый под палитру ACE-Step), курсив и моноширинный шрифт
 * — чтобы имя файла легко считывалось взглядом. */
.acestep-model-hint {
    color: rgba(216, 180, 254, 0.85);
    font-style: italic;
    font-size: 11px;
    letter-spacing: 0.1px;
}

/* Заголовок-разделитель секции body'а (AR / DiT / Выход): аппер-кейс, более
 * тусклый и разреженный — визуально группирует строки без тяжёлого бордюра. */
.acestep-section-header {
    color: rgba(216, 180, 254, 0.70);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.8px;
    text-transform: uppercase;
    margin-top: 4px;
}

@keyframes acestep-pulse {
    0%   { opacity: 0.60; }
    100% { opacity: 1.00; }
}
