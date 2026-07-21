/* Node card: карточка одной ноды в редакторе нод.
 * Стиль — тёмная карточка с тонкой границей и приподнятой тенью.
 * При выделении (.selected) рамка становится primary. */

/* Карточка ноды. Рамка реализована через inset box-shadow, а не через
 * настоящий border: DecoratedBox.post_build_display_list рисует border
 * ПОСЛЕ children, и любая точка порта оказалась бы под рамкой и выглядела
 * полупрозрачной. Inset shadow рендерится в build_display_list ДО детей —
 * children (PortDot'ы) рисуются поверх. */
.node-card {
    /* Карточка ноды shrink-to-fit'ится по контенту: header + body row.
       Width / height не задаются константой — Positioned пробрасывает loose
       constraints, measure_container активирует two-pass shrink-to-fit
       через `Dimension::FitContent` (syngui/src/mss/value.rs). */
    width: fit-content;
    height: fit-content;
    /* Цвет фона выносим в `--node-bg` (cascade), чтобы inline-style
       `--node-bg: <blended_tint>` мог per-instance переопределять цвет.
       Hover-state НЕ задаёт background-color → переменная остаётся
       той же при наведении и фон не мигает на дефолтный.
       Парсер syngui MSS не поддерживает fallback `var(--x, default)` —
       поэтому базовое значение задаём отдельным declaration'ом. */
    --node-bg: #2A2D38;
    background-color: var(--node-bg);
    border-radius: 8px;
    box-shadow:
        0 6px 16px rgba(0, 0, 0, 0.35),
        inset 0 0 0 1px rgba(255, 255, 255, 0.08);
    transition: box-shadow 180ms ease-out, transform 180ms ease-out;
}

.node-card:hover {
    box-shadow:
        0 10px 24px rgba(0, 0, 0, 0.45),
        inset 0 0 0 1px rgba(255, 255, 255, 0.18);
}

.node-card.selected {
    box-shadow:
        0 10px 24px rgba(0, 0, 0, 0.45),
        inset 0 0 0 2px var(--primary);
}

/* Drop-shadow выключен пользователем через ContextMenu «Тень: ☐»,
   или автоматически у disabled-ноды (см. ниже). Border (inset)
   сохраняем — без него карточка теряет визуальные границы.
   `:hover` override НУЖЕН: базовый `.node-card:hover` принудительно
   ставит drop-shadow при наведении — без явного override на
   `.no-shadow:hover` тень бы возвращалась при hover. */
.node-card.no-shadow {
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08);
}
.node-card.no-shadow:hover {
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.18);
}
.node-card.selected.no-shadow {
    box-shadow: inset 0 0 0 2px var(--primary);
}
.node-card.selected.no-shadow:hover {
    box-shadow: inset 0 0 0 2px var(--primary);
}

/* Disabled-нода: приглушена + всегда без drop-shadow. Все интерактивные
   элементы (drag по header'у, ContextMenu) продолжают работать, чтобы
   пользователь мог снова включить ноду. */
.node-card.disabled {
    opacity: 0.45;
    transition: opacity 180ms ease-out;
}

.node-card-icon {
    icon-size: 18px;
    color: var(--primary);
}

.node-card-title {
    color: #F0F1F4;
    font-size: 13px;
    font-weight: 600;
}

.node-card-close {
    color: #98A0AD;
    border-radius: 4px;
    transition: background-color 140ms ease-out, color 140ms ease-out;
}

.node-card-close:hover {
    background-color: rgba(255, 255, 255, 0.10);
    color: #FFFFFF;
}

.node-card-body {
    background-color: #232631;
    border-radius: 0 0 8px 8px;
    padding-top: 6px;
    padding-bottom: 6px;
}

.node-card-divider {
    height: 1px;
    background-color: rgba(255, 255, 255, 0.06);
    margin-top: 4px;
    margin-bottom: 4px;
}

/* Лейблы портов */
.node-card-port-label-in,
.node-card-port-label-out {
    color: #B7BDC9;
    font-size: 11px;
    font-weight: 500;
}

.node-card-port-label-out {
    text-align: right;
}

.node-card-port-spacer {
    flex-grow: 1;
    background-color: transparent;
}

/* Поля ноды */
.node-card-field-label {
    color: #98A0AD;
    font-size: 11px;
    font-weight: 500;
}

.node-card-field-spacer {
    flex-grow: 1;
    background-color: transparent;
}

.node-card-field-error {
    color: #E55353;
    font-size: 11px;
}

/* Вычисленные значения, показываемые внутри карточки. Output — крупно
 * и по центру; Add — мелко и приглушённо, выровнено по правому краю
 * под output-портом. */
.node-output-value {
    color: var(--primary);
    font-size: 18px;
    font-weight: 600;
    text-align: center;
    padding-top: 4px;
    padding-bottom: 4px;
}

.node-port-value {
    color: #98A0AD;
    font-size: 11px;
    font-weight: 500;
    text-align: right;
}
