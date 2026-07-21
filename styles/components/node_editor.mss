/* Node editor: viewport, world, toolbar.
 * Корневой контейнер заполняет shell-область, viewport имеет тёмно-нейтральный
 * фон в стилистике DaVinci Resolve. */

.node-editor-root {
    background-color: var(--bg-shell);
    width: 100%;
    height: 100%;
}

/* Canvas-area внутри page-Column: TabsBar сверху, всё остальное —
 * grow до конца (включая dot-grid). flex-grow:1 нужен чтобы canvas
 * не схлопывался в высоту нуля. */
.ne-canvas-area {
    background-color: var(--bg-shell);
    flex-grow: 1;
    width: 100%;
}

/* Wrapper-frame вокруг PanZoomViewport — даёт padding между сеткой
 * и границами shell'а (топбар сверху, nav_rail/templates-panel слева).
 * Border-radius закругляет canvas, inset-shadow добавляет глубины. */
.ne-canvas-frame {
    background-color: var(--bg-shell);
    padding: 2px 2px 2px 2px;
    width: 100%;
    height: 100%;
}

.node-editor-viewport {
    background-color: #1A1B22;
    border-radius: 12px;
    width: 100%;
    height: 100%;
    /* Цвет точек сетки (читается через mss.color у PanZoomViewport) */
    color: rgba(180, 190, 210, 0.18);
    box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.04),
                inset 0 0 24px rgba(0, 0, 0, 0.35);
}

/* ───────────────────────── Empty-state (нет вкладок) ──────────────── */
.ne-empty {
    background-color: var(--bg-shell);
    width: 100%;
    height: 100%;
}
.ne-empty-title {
    color: var(--text-muted);
    font-size: 16px;
    font-weight: 500;
}
.ne-empty-hint {
    color: var(--text-subtle);
    font-size: 13px;
}

.node-editor-world {
    background-color: transparent;
}

/* Toolbar — pill-стиль, идентичный run/pause/stop pill справа.
 * Фиксированная высота 36px (чтобы Row.cross-axis stretch не растягивал
 * на всю высоту canvas) + pill-radius + тот же фон/тень/border. */
.node-editor-toolbar {
    background-color: rgba(35, 37, 46, 0.92);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--radius-pill);
    height: 36px;
    box-shadow: 0 8px 22px rgba(0, 0, 0, 0.35);
    transition: background-color var(--duration-med) var(--ease-standard),
                box-shadow var(--duration-med) var(--ease-standard);
}

.node-editor-toolbar:hover {
    background-color: rgba(45, 47, 58, 0.94);
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.45);
}

.node-editor-toolbar-btn {
    color: #D8DCE4;
    background-color: transparent;
    border-radius: var(--radius-pill);
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.node-editor-toolbar-btn:hover {
    background-color: rgba(255, 255, 255, 0.08);
    color: #FFFFFF;
}

/* Хост-обёртка Reactive: даёт фиксированные constraints, иначе
 * Reactive.layout возьмёт min_width=0 (см. syngui reactive.rs:177-191)
 * и Text внутри схлопнется в 0×0. */
.node-editor-toolbar-zoom-host {
    width: 44px;
    height: 24px;
    background-color: transparent;
}

.node-editor-toolbar-zoom {
    color: #FFFFFF;
    font-size: 12px;
    font-weight: 600;
    text-align: center;
}
