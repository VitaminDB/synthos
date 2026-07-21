/* Стили для функциональных audio-нод (AudioFile / AudioPlayer / AudioRecorder).
 *
 * Палитра наследована из node_editor_node.mss:
 *   карточка        #2A2D38
 *   body            #232631
 *   text-strong     #F0F1F4
 *   port label      #B7BDC9
 *   muted text      #98A0AD
 *   accent          var(--primary)  (#EE5E48)
 *
 * Audio-ноды — широкие плоские карточки. Body = одна Row с
 * controls слева, waveform-strip справа. */

/* Боковой отступ внутри body-row, заменяет внешний Padding::symmetric.
 * Audio-body — единый Row с PortDot’ами на краях, поэтому слева/справа
 * нельзя обернуть в Padding (тогда точка не дотянется до края карточки).
 * `audio-node-side-pad` — пустой DecoratedBox 10px без bg/border. */
.audio-node-side-pad {
    background-color: transparent;
    width: 10px;
    height: 1px;
}

/* ── strip-волна ──────────────────────────────────────────────────────── */
.audio-node-waveform-host {
    background-color: transparent;
    flex-grow: 1;
    min-width: 120px;
    max-height: 40px;
    height: 40px;
}

.audio-node-waveform {
    background-color: #1B1D26;
    color: #6B7384;
    accent-color: var(--primary);
    border-radius: 4px;
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.04);
    padding: 2px;
    transition: box-shadow 140ms ease-out;
}

.audio-node-waveform:hover {
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.10);
}

.audio-node-waveform-empty {
    background-color: #1B1D26;
    color: #6B7384;
    border-radius: 4px;
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.04);
    min-height: 40px;
}

.audio-node-waveform-placeholder {
    color: #6B7384;
    font-size: 11px;
    font-style: italic;
}

.audio-node-waveform-live {
    background-color: #1B1D26;
    color: var(--primary);
    accent-color: var(--primary);
    border-radius: 4px;
    box-shadow: inset 0 0 0 1px rgba(238, 94, 72, 0.20);
    padding: 2px;
}

/* ── текстовая мета (filename, sr/ch, status) ─────────────────────────── */
.audio-node-info {
    background-color: transparent;
    min-width: 100px;
    max-width: 140px;
    /* Высота shrink-to-fit'ится строго по содержимому: одна строка =
       ~16px, длинное имя с переносами = больше. Без `min-height` чтобы
       Row CrossAxisAlignment::Center корректно выравнивал info-Column
       по центру относительно folder-icon (40px) — иначе при single-line
       тексте info=40 принудительно top-align'ит Text. Базовая высота
       строки карточки удерживается через `.audio-node-body-host
       { min-height: 56px }`. */
    height: fit-content;
}

/* Body-host: фиксированная высота вне зависимости от tight-constraints
 * родителя (Column/Stack растягиваются до max_height; DecoratedBox с MSS
 * height — нет). Высота = waveform 40 + padding 8 + 8 = 56.
 *
 * Без `overflow: hidden` — иначе обрезаются «выглядывающие» половинки
 * PortDot’ов, которые `set_position` смещает на ±PORT_DOT/2 за пределы
 * body-host bounds. Превышение по ширине предотвращается flex-shrink’ом
 * у `.audio-node-waveform-host` (есть `flex-grow: 1`, нет `min-width`-
 * блокеров для других widget’ов). */
.audio-node-body-host {
    background-color: transparent;
    /* Высота shrink-to-fit'ится по контенту Row'а — typical 40px (icon-btn
       + waveform). Без min-height: иначе Row top-aligned внутри 56px-блока
       создавал асимметричный 16px-«хвост» снизу карточки. Симметричный
       вертикальный padding обеспечивает `.node-card-body` (padding 6+6). */
    height: fit-content;
}

/* Per-node ширины: каждая audio-нода имеет свою фиксированную ширину
   (раньше — `NodeKindMeta::fixed_width`). Теперь это явный MSS-property
   на host-DecoratedBox, что отражает реальный CSS-дизайн (контейнер сам
   декларирует width). `flex-spacer` (.audio-node-flex-spacer) распределяет
   остаток в этой ширине. Карточка `.node-card` shrink-to-fit'ится к этой
   width + header. */
.audio-file-host     { width: 480px; }
.audio-recorder-host { width: 460px; }
.audio-player-host   { width: 540px; }
.audio-save-host     { width: 520px; }
.audio-gain-host     { width: 360px; }
.audio-filter-host   { width: 420px; }
.audio-reverb-host   { width: 420px; }

/* Все «реактивные» слоты (Reactive внутри Row) — с фиксированной высотой,
 * чтобы Reactive не растягивался до max_height родителя. */
.audio-node-slot {
    background-color: transparent;
    height: 40px;
    max-height: 40px;
}

.audio-node-slot-btn {
    background-color: transparent;
    width: 32px;
    height: 32px;
    max-height: 32px;
}

.audio-node-slot-text {
    background-color: transparent;
    height: 20px;
    max-height: 20px;
}

.audio-node-filename {
    color: #F0F1F4;
    font-size: 12px;
    font-weight: 600;
}

.audio-node-meta {
    color: #98A0AD;
    font-size: 10px;
}

.audio-node-empty {
    color: #6B7384;
    font-size: 11px;
    font-style: italic;
}

.audio-node-status {
    color: #98A0AD;
    font-size: 10px;
}

.audio-node-status-active {
    color: var(--primary);
    font-size: 10px;
    font-weight: 600;
}

.audio-node-error {
    color: #E55353;
    font-size: 10px;
}

.audio-node-format-icon {
    color: #98A0AD;
    icon-size: 16px;
}

/* ── transport-buttons (Play / Pause / Stop / Record) ─────────────────── */
.audio-node-transport-btn {
    background-color: rgba(255, 255, 255, 0.04);
    color: #F0F1F4;
    border-radius: 14px;
    transition: background-color 140ms ease-out, color 140ms ease-out, transform 100ms ease-out;
}

.audio-node-transport-btn:hover {
    background-color: rgba(255, 255, 255, 0.10);
}

.audio-node-transport-btn:active {
    transform: scale(0.94);
}

.audio-node-play {
    background-color: var(--primary);
    color: #FFFFFF;
}

.audio-node-play:hover {
    background-color: var(--primary-hover);
}

.audio-node-pause {
    background-color: #EAB308;
    color: #1F2937;
}

.audio-node-pause:hover {
    background-color: #FBBF24;
}

.audio-node-stop {
    background-color: rgba(255, 255, 255, 0.04);
    color: #B7BDC9;
}

.audio-node-stop:hover {
    background-color: rgba(255, 255, 255, 0.10);
    color: #F0F1F4;
}

.audio-node-record {
    background-color: #B91C1C;
    color: #FFFFFF;
}

.audio-node-record:hover {
    background-color: #DC2626;
}

/* Pulse-анимация для активной кнопки записи: красное свечение пульсирует. */
@keyframes audio-record-pulse {
    0%   { background-color: #DC2626; box-shadow: 0 0 0 0 rgba(220, 38, 38, 0.55); }
    70%  { background-color: #B91C1C; box-shadow: 0 0 0 5px rgba(220, 38, 38, 0.0); }
    100% { background-color: #DC2626; box-shadow: 0 0 0 0 rgba(220, 38, 38, 0.0); }
}

.audio-node-record-active {
    background-color: #DC2626;
    color: #FFFFFF;
    animation: audio-record-pulse 1.4s infinite ease-in-out;
}

/* ── volume slider (узкий компактный) ─────────────────────────────────── */
.audio-node-volume-slider {
    width: 100px;
    background-color: rgba(255, 255, 255, 0.06);
    color: var(--primary);
    accent-color: #FFFFFF;
    border-color: var(--primary);
    border-radius: 2px;
    min-height: 3px;
    max-height: 12px;
}

.audio-node-volume-icon {
    color: #98A0AD;
    icon-size: 14px;
}

/* ── LIVE-бейдж в waveform-области (streaming-режим Player'а) ─────────── */
@keyframes audio-live-pulse {
    0%   { color: #FF6B5C; }
    50%  { color: #FFB3AA; }
    100% { color: #FF6B5C; }
}

.audio-node-waveform-live-host {
    background-color: #1B1D26;
    color: #6B7384;
    border-radius: 4px;
    box-shadow: inset 0 0 0 1px rgba(255, 107, 92, 0.20);
    flex-grow: 1;
    min-width: 120px;
    max-height: 40px;
    height: 40px;
}

.audio-node-live-badge {
    color: #FF6B5C;
    icon-color: #FF6B5C;
    icon-size: 14px;
    font-weight: 600;
    letter-spacing: 1.5px;
    animation: audio-live-pulse 1.6s infinite ease-in-out;
}

/* ── timecode + open-button ───────────────────────────────────────────── */
.audio-node-timecode {
    color: #B7BDC9;
    font-size: 10px;
    font-weight: 500;
}

.audio-node-open-btn {
    background-color: rgba(255, 255, 255, 0.06);
    color: #B7BDC9;
    border-radius: 6px;
    transition: background-color 140ms ease-out, color 140ms ease-out;
}

.audio-node-open-btn:hover {
    background-color: rgba(238, 94, 72, 0.15);
    color: var(--primary);
}

/* ── Effect-ноды: Gain / Filter / Reverb ──────────────────────────────────
 * Слайдеры компактные, в стилистике audio-node-volume-slider.
 * .filter-node-mode-host обёртывает Dropdown — фиксируем высоту,
 * чтобы dropdown не растягивал body. */

.gain-node-slider,
.filter-node-slider,
.reverb-node-slider {
    width: 110px;
    background-color: rgba(255, 255, 255, 0.06);
    color: var(--primary);
    accent-color: #FFFFFF;
    border-color: var(--primary);
    border-radius: 2px;
    min-height: 3px;
    max-height: 12px;
}

/* Flex spacer для эффект-нод. Растягивает Row до полной ширины
 * body-host'а — без него output PortDot прижимается к контентам и
 * остаётся в середине карточки, оставляя справа пустую полосу. */
.audio-node-flex-spacer {
    background-color: transparent;
    flex-grow: 1;
    min-width: 1px;
    height: 1px;
}

.filter-node-mode-host {
    background-color: transparent;
    width: 64px;
    height: 28px;
    max-height: 28px;
}

/* Dropdown LP/HP/BP — тёмный bg, согласованный с .node-input-dropdown
 * (см. node_editor_inputs.mss:74). */
.filter-node-dropdown {
    background-color: #1B1D26;
    color: #E6E9EF;
    border-radius: 6px;
    border-color: rgba(255, 255, 255, 0.10);
    border-width: 1px;
    width: 64px;
    height: 28px;
    max-height: 28px;
    font-size: 11px;
    font-weight: 600;
    padding-left: 6px;
    padding-right: 6px;
    transition: border-color 160ms ease-out, background-color 160ms ease-out;
}

.filter-node-dropdown:hover {
    border-color: rgba(255, 255, 255, 0.20);
}

.filter-node-dropdown:focus {
    border-color: var(--primary);
}

/* ── Save-to-File нода ──────────────────────────────────────────────────── */

.save-node-path-host {
    background-color: transparent;
    width: 280px;
    height: 32px;
    max-height: 32px;
}

.save-node-status-host {
    background-color: transparent;
    min-width: 140px;
    max-width: 220px;
    height: 20px;
    max-height: 20px;
}

.save-node-status {
    color: #98A0AD;
    font-size: 10px;
    transition: color 200ms ease-out;
}

.save-node-status-active {
    color: var(--primary);
    font-size: 10px;
    font-weight: 600;
    /* Лёгкая «дышащая» прозрачность пока идёт запись — pulse общий с record. */
    animation: audio-record-pulse 1.4s infinite ease-in-out;
}

.save-node-status-saved {
    color: #34D399; /* emerald-400 */
    font-size: 10px;
    font-weight: 600;
}

/* ── Mixer-нода ─────────────────────────────────────────────────────────── */

/* Контейнер всей карточки. Базовый body-host задаёт серую рамку; мы
   просто фиксируем ширину так, чтобы микшер влазил в один экран при
   минимальном N=2 без раздражающих переносов. Дальше width растёт сам
   через child Reactive — Column/Row из syngui shrink-to-fit. */
/* Не задаём горизонтальный padding: input/output PortDot'ы должны касаться
   левой/правой границы карточки половиной круга (см. PortDotElement::
   set_position со смещением ±PORT_DOT/2). Вертикальный padding наследуется
   от `.node-card-body { padding: 6px 0 }`. */
.audio-mixer-host {
    min-width: 360px;
    max-width: 460px;
}

/* Один канал-stripe: полупрозрачная подложка для визуальной группировки
   row'ов между собой. Hover слегка подчёркивает, чтобы при перетаскивании
   wires пользователь видел границу row. Без horizontal padding — иначе
   input PortDot сдвигается внутрь карточки и теряет «прикус» к рамке. */
.mixer-channel-row {
    background-color: rgba(255, 255, 255, 0.025);
    border-radius: 4px;
    transition: background-color 160ms ease-out;
}

.mixer-channel-row:hover {
    background-color: rgba(255, 255, 255, 0.045);
}

/* Индекс канала — мелкий моноширинный счётчик. Чтобы у разных N
   ширина row'а не «дёргалась», даём min-width=14px. */
.mixer-channel-index {
    color: #98A0AD;
    font-size: 10px;
    font-weight: 600;
    min-width: 14px;
}

/* Slider растягивается на оставшееся пространство channel-row через
   flex-grow. min-width — нижняя граница, чтобы при узкой карточке
   слайдер не схлопывался в ноль. */
.mixer-channel-slider {
    flex-grow: 1;
    min-width: 120px;
}

/* Левая колонка (header + channels) занимает всё доступное пространство
   body-row — output PortDot сам прижимается к правому краю последним
   ребёнком. */
.mixer-inner-col {
    flex-grow: 1;
    min-width: 1px;
}

/* Компактный SpinBox в header'е микшера (выбор количества входов).
   padding-left используется spin_box.rs как ширина минус-/плюс- кнопок —
   16px на каждую + 12px на value-area даёт суммарную ширину 44px. */
.mixer-input-count {
    background-color: #1B1D26;
    color: #E6E9EF;
    border-color: rgba(255, 255, 255, 0.10);
    border-width: 1px;
    border-radius: 6px;
    height: 24px;
    font-size: 11px;
    padding-left: 18px;
    transition: border-color 160ms ease-out;
}

.mixer-input-count:hover {
    border-color: rgba(255, 255, 255, 0.20);
}

.mixer-input-count:focus {
    border-color: var(--primary);
}

/* ── Общие классы для controls/ ──────────────────────────────────────────
 * Используются нодами через app/synthos/src/pages/node_editor/controls/.
 * `node_field_row`, `node_slider_field`, `node_dropdown_field`,
 * `node_file_picker`, `node_timecode`, `node_transport_buttons`,
 * `node_status_text`. */

.node-field-row {
    padding-left: 0px;
    padding-right: 0px;
    padding-top: 0px;
    padding-bottom: 0px;
}

.node-field-label-cell {
    flex-grow: 3;
    padding-left: 10px;
    padding-right: 10px;
    padding-top: 2px;
    padding-bottom: 2px;
}

.node-field-control-cell {
    flex-grow: 7;
    padding-left: 10px;
    padding-right: 10px;
    padding-top: 2px;
    padding-bottom: 2px;
}

.node-slider-stretch {
    width: auto;
    flex-grow: 1;
}

.node-slider-readout {
    color: #98A0AD;
    font-size: 11px;
    font-weight: 500;
    min-width: 38px;
}

.node-file-picker-row {
    padding-left: 0px;
    padding-right: 0px;
}

.node-file-picker-btn {
    background-color: rgba(255, 255, 255, 0.06);
    color: #E6E9EF;
    border-radius: 6px;
    width: 28px;
    height: 28px;
    font-size: 16px;
    transition: background-color 160ms ease-out;
}

.node-file-picker-btn:hover {
    background-color: rgba(255, 255, 255, 0.12);
}

.node-file-picker-name {
    color: #E6E9EF;
    font-size: 11px;
    font-weight: 500;
}

.node-file-picker-empty {
    color: #98A0AD;
    font-size: 11px;
    font-weight: 400;
}

.node-timecode {
    color: #98A0AD;
    font-size: 11px;
    font-weight: 500;
    font-family: monospace;
}

.node-transport-row {
    padding-left: 0px;
    padding-right: 0px;
}

.node-transport-btn {
    background-color: rgba(255, 255, 255, 0.06);
    color: #E6E9EF;
    border-radius: 6px;
    width: 32px;
    height: 32px;
    font-size: 18px;
    transition: background-color 160ms ease-out, color 160ms ease-out;
}

.node-transport-btn:hover {
    background-color: rgba(255, 255, 255, 0.12);
}

.node-transport-btn:active {
    background-color: rgba(255, 255, 255, 0.18);
}

.node-transport-play {
    color: #6DD3A8;
}

.node-transport-pause {
    color: #F2B85F;
}

.node-transport-stop {
    color: #E07A6A;
}

.node-status-idle {
    color: #6F7682;
    font-size: 11px;
}

.node-status-running {
    color: #6DD3A8;
    font-size: 11px;
    font-weight: 500;
}

.node-status-error {
    color: #E07A6A;
    font-size: 11px;
    font-weight: 500;
}

.node-status-done {
    color: #98A0AD;
    font-size: 11px;
}

.node-card-port-col {
    padding-left: 0px;
    padding-right: 0px;
}

.node-card-port-col-empty {
    background-color: transparent;
    width: 0px;
    height: 0px;
}

