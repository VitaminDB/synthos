/* ── Equalizer node ───────────────────────────────────────────────────────
 * Graphic-EQ ноды (6/10/20/30 полос). Тело:
 *   [in_dot] [reset_btn] [<col_band>...] [out_dot]
 * Каждая колонка — gain_label / vertical-slider / freq_label.
 * ─────────────────────────────────────────────────────────────────────── */

.equalizer-node-host {
    /* Высота shrink-to-fit'ится по Row{reset, columns} (= 156px от
       `.equalizer-band-column`). Без `min-height` — иначе пустое
       пространство сверху/снизу row сдвигало бы port'ы от центра
       визуального содержимого относительно `port_world_pos`-формулы
       (которая вычисляет `(HEADER + card.height) / 2`). */
    height: fit-content;
    background-color: transparent;
}

/* Reset-кнопка */
.equalizer-reset-btn {
    width: 28px;
    height: 28px;
    border-radius: 14px;
    background-color: rgba(255, 255, 255, 0.04);
    color: #DCDFE5;
    transition: background-color 160ms ease-out, color 160ms ease-out;
}
.equalizer-reset-btn:hover {
    background-color: rgba(255, 255, 255, 0.10);
    color: #FFFFFF;
}

/* Столбец одной полосы */
.equalizer-band-column {
    width: 36px;
    /* Высота shrink-to-fit'ится по содержимому (gain-label/slider/freq-label);
       min-height обеспечивает базовую высоту slider-track'а. */
    height: fit-content;
    min-height: 156px;
    background-color: transparent;
}

/* Хосты для текстовых лейблов (фиксируют высоту, чтобы Reactive-rebuild
   не дёргал layout слайдера). */
.equalizer-band-gain-host {
    height: 14px;
    background-color: transparent;
}
.equalizer-band-freq-host {
    height: 14px;
    background-color: transparent;
}

/* Вертикальный слайдер одной полосы.
   Для Slider в vertical-режиме:
     width  — общая толщина карты (включая thumb-rect)
     height — длина шкалы
     min-width  — толщина track'а (узкая полоса)
     max-width  — ширина thumb'а
     background-color — фон track'а
     color            — fill (от центра 0 dB)
     accent-color     — заливка thumb'а
     border-color     — рамка thumb'а */
.equalizer-band-slider {
    width: 24px;
    height: 120px;
    min-width: 4px;
    max-width: 18px;
    background-color: rgba(255, 255, 255, 0.08);
    color: var(--primary);
    accent-color: #FFFFFF;
    border-color: var(--primary);
    border-radius: 2px;
    border-width: 1px;
}

.equalizer-band-gain-label {
    font-size: 10px;
    color: #DCDFE5;
    text-align: center;
}
.equalizer-band-freq-label {
    font-size: 10px;
    color: rgba(220, 223, 229, 0.65);
    text-align: center;
}

/* Сжатие столбцов при росте n_bands — экономия ширины карточки */
.eq-20 .equalizer-band-column {
    width: 30px;
}
.eq-20 .equalizer-band-slider {
    width: 20px;
    max-width: 14px;
}
.eq-30 .equalizer-band-column {
    width: 28px;
}
.eq-30 .equalizer-band-slider {
    width: 18px;
    max-width: 12px;
    min-width: 3px;
}
