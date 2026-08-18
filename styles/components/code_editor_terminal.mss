/* ───────────────────────── VTE-терминал в редакторе кода ─────────────
 * Стилизация виджета syngui::widgets::Terminal на странице «Редактор
 * кода», его gear-кнопки настроек шрифта и popover'а с панелью.
 *
 * Терминал использует ту же светлую поверхность `var(--bg-panel)`, что
 * file tree и редактор — чтобы не выпадать из общей темы synthos.
 * Текст — `var(--text)`. ANSI bright-цвета (через ESC-последовательности
 * powerlevel10k и подобных prompt'ов) остаются цветными как и были —
 * они не контролируются MSS, а приходят как RGB из VTE.
 *
 * Все радиусы/тайминги — через дизайн-токены variables.mss; никаких
 * хардкод-значений. Анимации — keyframes + transitions, как требует
 * TASK.md «всё оформление через mss с анимациями и transitions».
 * ──────────────────────────────────────────────────────────────────── */

/* ───────────────────────── Сам терминал ────────────────────────────── */

/*
 * font-family / font-size НЕ задаём в MSS: они приходят из RwSignal через
 * builder (`AppCtx.terminal_font_family/_size`), persist'ятся в конфиг и
 * меняются gear-popover'ом. apply_computed_style вызывается ПОСЛЕ update
 * в render-цикле и перезаписал бы builder-значения, обнуляя пользовательские
 * настройки на каждом кадре.
 */
/* padding-right = ширина scrollbar (10px) + зазор: полоса рисуется поверх
 * bounds у правого края, а сетку ячеек терминал считает уже по padding'у —
 * без запаса thumb перекрывал бы последнюю колонку вывода. */
.code-editor-terminal {
    background-color: var(--bg-panel);
    color: var(--text);
    padding: 4px 16px 4px 4px;
}

/* ───────────────────────── Gear-кнопка (правый верх) ────────────────── */

/*
 * Полупрозрачная заливка над cell-сеткой; на hover — заметно ярче,
 * чтобы пользователь понимал, что это интерактивный элемент. Иконка
 * MI_TUNE из Material Icons (по правилу TASK.md «всегда material icons»).
 */
.term-gear-btn {
    color: #8B95A3;
    background-color: rgba(20, 24, 30, 0.55);
    border-radius: var(--radius-pill);
    padding: 6px 6px 6px 6px;
    icon-size: 18px;
    cursor: pointer;
    opacity: 0.7;
    transition: opacity var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard),
                background-color var(--duration-fast) var(--ease-standard);
}
.term-gear-btn:hover {
    opacity: 1.0;
    color: #FFFFFF;
    background-color: rgba(50, 58, 70, 0.85);
}
.term-gear-btn:active {
    background-color: rgba(70, 80, 96, 0.95);
}

/* ───────────────────────── Gear-popover (Portal) ───────────────────── */

/*
 * Плавающая карточка с панелью настроек шрифта. Открывается gear-кнопкой
 * через Portal (PortalAnchor::TopEnd, margin 16/56). Появление
 * анимируется keyframe'ами `term-gear-pop-in` — лёгкий scale + fade.
 */
.term-gear-popover {
    background-color: var(--bg-panel);
    border-width: 1px;
    border-color: var(--border);
    border-radius: var(--radius-panel);
    padding: 16px 16px 16px 16px;
    box-shadow: 0 10px 30px rgba(15, 23, 42, 0.18);
    /*
     * 560px — компромисс между «не закрывает терминал целиком» и «хватает
     * на settings-row layout»: иконка(40) + gap(16) + grow-блок с
     * заголовком и описанием + gap(16) + control. При 420px Dropdown
     * (220px min) задушивал текст-блок до одной буквы ширины (видно было
     * на первом скриншоте).
     */
    width: 560px;
    animation: term-gear-pop-in var(--duration-med) var(--ease-standard);
}

@keyframes term-gear-pop-in {
    from {
        opacity: 0;
    }
    to {
        opacity: 1;
    }
}

/* ───────────────────────── Settings → Терминал (страница) ────────────── */

.settings-page.terminal-page {
    background-color: var(--bg-panel);
}

/*
 * `.terminal-settings-panel` — общий контейнер панели, переиспользуется
 * и в Settings, и внутри popover'а. Padding=0, чтобы внешний контейнер
 * сам решал отступы (Settings → 32px, popover → 16px из MSS popover'а).
 */
.terminal-settings-panel {
    background-color: transparent;
}

/* ───────────────────────── Контролы выбора шрифта ───────────────────── */

.terminal-font-control {
    background-color: transparent;
}

.terminal-font-dropdown {
    min-width: 180px;
}

/* Wrap для слайдера — в Row просто dt-Slider схлопывается до 0px width
 * (`width.unwrap_or(constraints.max_width)` без явной width даёт ребёнку
 * intrinsic-расчёт = 0). Ставим явный отступ слева/справа, чтобы trekк
 * не упирался в края settings-row. */
.terminal-font-slider-wrap {
    padding: 4px 4px 4px 0px;
}

/* Цвета трека/заливки/ползунка приходят глобальным правилом `Slider` из
 * base/reset.mss — здесь только геометрия. */
.terminal-font-slider {
    /* min-width — на всякий случай: некоторые layout-paths могут не
     * передать flex-grow в DecoratedBox-обёртку. */
    min-width: 200px;
    height: 24px;
}

.terminal-font-size-value {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
    min-width: 56px;
}

/* ───────────────────────── Live preview ────────────────────────────── */

/*
 * Превью повторяет цветовую схему терминала, чтобы пользователь видел
 * шрифт ровно так, как он отрисуется в реальной cell-сетке.
 * Замечание: `font-family`/`font-size` здесь специально НЕ заданы —
 * эти свойства приходят на дочерние Text через MSS-наследование от
 * RwSignal terminal_font_*: closure внутри Reactive переризует preview
 * с актуальным шрифтом (см. panel.rs::preview_card).
 */
.terminal-preview-card {
    background-color: var(--bg-panel);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border);
    transition: border-color var(--duration-fast) var(--ease-standard);
}
.terminal-preview-card:hover {
    border-color: var(--border-strong);
}

.terminal-preview-line {
    color: var(--text);
    font-family: monospace;
    font-size: 13px;
}

.terminal-preview-caption-wrap {
    background-color: transparent;
    border-top-width: 1px;
    border-color: var(--border);
    padding: 8px 0px 0px 0px;
}

.terminal-preview-caption {
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 500;
}
