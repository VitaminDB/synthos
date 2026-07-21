/* Раздел настроек «Модели» — редактор пресетов llama.cpp + правая панель. */

/* ───────────────────────── Общие контейнеры ───────────────────────── */

.models-page {
    background-color: var(--bg-chats);
    padding: 0px;
}

.models-empty-bubble {
    width: 96px;
    height: 96px;
    border-radius: 48px;
    background-color: var(--primary-soft);
    transition: transform var(--duration-med) var(--ease-standard),
                background-color var(--duration-med) var(--ease-standard);
}

.models-empty-bubble:hover {
    transform: scale(1.04);
}

.models-empty-icon {
    color: var(--primary);
    icon-size: 44px;
}

.models-empty-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--text);
}

.models-empty-text {
    font-size: 13px;
    color: var(--text-muted);
    text-align: center;
    line-height: 1.5;
}

/* ───────────────────────── Заголовок-карточка (имя модели) ───────────────────────── */

/* Зеркалит .settings-card (layout/settings.mss): одинаковые border-color
 * и radius, чтобы карточки «Имя модели» и «Пути» выглядели одинаково. */
.models-card {
    background-color: var(--bg-panel);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border);
    transition: border-color var(--duration-med) var(--ease-standard);
}

.models-card-header {
    background-color: var(--bg-panel);
}

.models-header-icon-wrap {
    width: 48px;
    height: 48px;
    border-radius: 14px;
    background-color: var(--primary-soft);
    transition: transform var(--duration-med) var(--ease-standard);
}

.models-header-icon-wrap:hover {
    transform: rotate(-8deg);
}

.models-header-icon {
    color: var(--primary);
    icon-size: 26px;
}

.models-field-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    letter-spacing: 0.04em;
    text-transform: uppercase;
}

.models-name-field {
    background-color: var(--bg-chat);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    padding-left: 14px;
    padding-right: 14px;
    height: 42px;
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
    transition: border-color var(--duration-fast) var(--ease-standard);
}

.models-name-field:focus {
    border-color: var(--primary);
}

.models-delete-btn {
    background-color: transparent;
    color: var(--text-muted);
    border-radius: 10px;
    padding-left: 14px;
    padding-right: 14px;
    height: 38px;
    font-size: 13px;
    font-weight: 600;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.models-delete-btn:hover {
    background-color: #FFE9E5;
    color: #C0392B;
}

/* ───────────────────────── Поля-пути ───────────────────────── */

.models-path-field {
    background-color: var(--bg-chat);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    padding-left: 12px;
    padding-right: 12px;
    height: 38px;
    width: 360px;
    font-size: 13px;
    color: var(--text);
    transition: border-color var(--duration-fast) var(--ease-standard);
}

.models-path-field:focus {
    border-color: var(--primary);
}

.models-path-browse {
    background-color: var(--primary-soft);
    color: var(--primary);
    border-radius: 10px;
    width: 38px;
    height: 38px;
    icon-size: 20px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                color            var(--duration-fast) var(--ease-standard);
}

.models-path-browse:hover {
    background-color: var(--primary);
    color: var(--on-primary);
}

/* ───────────────────────── Активные настройки ───────────────────────── */

/* Разделитель строк. Clip по радиусу карты делает overflow: hidden на
 * `.settings-card` — без него нижний border торчит за скруглённый угол. */
.models-active-row {
    background-color: transparent;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.models-active-chip {
    background-color: var(--primary);
    color: var(--on-primary);
    border-radius: 16px;
    height: 32px;
    padding-left: 14px;
    padding-right: 10px;
    font-size: 12px;
    font-weight: 600;
    icon-size: 16px;
    accent-color: var(--primary);
    transition: background-color var(--duration-fast) var(--ease-standard),
                transform var(--duration-fast) var(--ease-standard);
}

.models-active-chip:hover {
    background-color: var(--primary-hover);
    transform: translateY(-1px);
}

.models-active-control {
    background-color: var(--bg-chat);
    border-color: var(--border-soft);
    color: var(--text);
    border-radius: 10px;
    height: 36px;
    accent-color: var(--primary);
    transition: border-color var(--duration-fast) var(--ease-standard);
}

.models-active-textfield {
    background-color: var(--bg-chat);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    padding-left: 12px;
    padding-right: 12px;
    height: 36px;
    width: 220px;
    font-size: 13px;
    color: var(--text);
    transition: border-color var(--duration-fast) var(--ease-standard);
}

.models-active-textfield:focus {
    border-color: var(--primary);
}

.models-active-dropdown {
    background-color: var(--bg-chat);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    height: 36px;
    min-width: 140px;
    font-size: 13px;
    color: var(--text);
    transition: border-color var(--duration-fast) var(--ease-standard);
}

.models-active-dropdown:hover {
    border-color: var(--border-strong);
}

/* ───────────────────────── Divider ───────────────────────── */

.models-divider {
    color: var(--border-soft);
    margin-top: 8px;
    margin-bottom: 8px;
}

/* ───────────────────────── Доступные настройки ───────────────────────── */

.models-available-wrap {
    background-color: transparent;
}

.models-available-hint {
    font-size: 12px;
    color: var(--text-muted);
    line-height: 1.5;
}

.models-available-category {
    font-size: 12px;
    font-weight: 700;
    color: var(--text-muted);
    letter-spacing: 0.05em;
    text-transform: uppercase;
    margin-top: 4px;
}

.models-available-chip {
    background-color: var(--bg-search);
    border-width: 1px;
    border-color: var(--border-strong);
    color: var(--text);
    border-radius: 18px;
    height: 34px;
    padding-left: 12px;
    padding-right: 14px;
    font-size: 12px;
    font-weight: 500;
    icon-size: 14px;
    accent-color: var(--primary);
    transition: background-color var(--duration-fast) var(--ease-standard),
                border-color   var(--duration-fast) var(--ease-standard),
                color          var(--duration-fast) var(--ease-standard),
                transform      var(--duration-fast) var(--ease-standard),
                box-shadow     var(--duration-fast) var(--ease-standard);
}

.models-available-chip:hover {
    background-color: var(--primary-soft);
    border-color: var(--primary);
    color: var(--primary);
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(238, 94, 72, 0.16);
}

/* ───────────────────────── Tooltip ───────────────────────── */

/* Фон, padding и тень — через MSS. После фикса LayoutHint::Tooltip в syngui
 * padding_* передаются в position layout и реально смещают контент от
 * фона. Shadow рисуется TooltipElement только при явном box-shadow. */
.models-chip-tooltip {
    background-color: #1F2029;
    border-radius: 12px;
    border-width: 0px;
    padding-left: 14px;
    padding-right: 14px;
    padding-top: 10px;
    padding-bottom: 12px;
    box-shadow: 0 6px 18px rgba(14, 14, 26, 0.22);
}

.models-tip-title {
    font-size: 13px;
    font-weight: 700;
    color: #FFFFFF;
    line-height: 1.3;
}

.models-tip-cli {
    font-size: 11px;
    font-weight: 600;
    color: #FFBFA8;
    font-family: monospace;
    line-height: 1.4;
}

.models-tip-desc {
    font-size: 12px;
    color: #D9DBE3;
    line-height: 1.55;
}

.models-tip-env {
    font-size: 10px;
    font-weight: 600;
    color: #8A8F9E;
    letter-spacing: 0.06em;
    font-family: monospace;
    line-height: 1.4;
}

/* ───────────────────────── Правая панель ───────────────────────── */

.models-panel {
    background-color: var(--bg-panel);
}

.models-add-btn {
    background-color: var(--primary-soft);
    color: var(--primary);
    border-radius: 10px;
    width: 36px;
    height: 36px;
    icon-size: 20px;
    transition: background-color var(--duration-fast) var(--ease-standard),
                transform       var(--duration-fast) var(--ease-standard);
}

.models-add-btn:hover {
    background-color: var(--primary);
    color: var(--on-primary);
    transform: rotate(90deg);
}

.models-empty-list {
    font-size: 12px;
    color: var(--text-muted);
    text-align: center;
    line-height: 1.5;
}
