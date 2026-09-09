/* Defaults. Note: MSS does not cascade `color` from parent to Text children,
 * so every Text gets an explicit class. Keep these defaults conservative. */

Text {
    color: var(--text);
    font-size: 14px;
}

Icon {
    color: var(--text-muted);
    icon-size: 20px;
}

/* TextField — фиксированная ширина 240px (без неё виджет жадно растягивается
 * на всю доступную ширину Row и вылезает за settings-card). `--bg-search`
 * фон совпадает с Dropdown/SpinBox, чтобы все form-контролы сидели в одной
 * визуальной линии. */
TextField {
    width: 240px;
    background-color: var(--bg-search);
    color: var(--text);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    accent-color: var(--primary);
    padding: 10px 14px;
    font-size: 14px;
    transition: border-color var(--duration-fast) var(--ease-standard);
}

TextField:focus {
    border-color: var(--primary);
}

/* Toggle — глобально привязан к теме приложения. Track OFF на `--bg-search`
 * (тот же фон, что у других form-контролов), track ON — accent темы; thumb
 * красится в `--bg-shell`, чтобы оставаться контрастным на любой палитре
 * (вместо хардкод-белого, который сливался в светлых темах с track ON и
 * выглядел грязно в тёмных). */
Toggle {
    background-color: var(--bg-search);
    border-color: var(--border);
    color: var(--bg-shell);
    accent-color: var(--primary);
    transition: background-color var(--duration-fast) var(--ease-standard);
}

/* Checkbox — привязан к теме приложения. Без этого правила виджет берёт
 * свои хардкод-дефолты (белая заливка, серая рамка #D1D5DB) и в тёмной теме
 * выглядит белым квадратом. `background-color` — фон невыбранной коробки,
 * `accent-color` — заливка выбранной, `color` — подпись; галочка внутри
 * всегда белая (checkbox.rs), и на `--primary` она контрастна в любой
 * палитре. */
Checkbox {
    background-color: var(--bg-search);
    border-color: var(--border);
    color: var(--text);
    accent-color: var(--primary);
    border-width: 2px;
    border-radius: 4px;
    font-size: 14px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

/* SpinBox — единый визуальный набор с TextField/Dropdown: `--bg-search` фон,
 * `--border-soft` бордер, 10px радиус. `accent-color` красит рамку в режиме
 * редактирования (см. spin_box.rs::build_display_list). */
SpinBox {
    background-color: var(--bg-search);
    color: var(--text);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    accent-color: var(--primary);
    font-size: 14px;
    transition: border-color var(--duration-fast) var(--ease-standard);
}

SpinBox:hover {
    border-color: var(--primary);
}

/* Dropdown — единый набор с TextField/SpinBox. Глобальное правило, а не
 * только `.settings-row-dropdown`: без него Dropdown'ы без класса (в
 * настройках аудио-моделей их с десяток) падали на встроенные дефолты
 * syngui — светлая заливка и светлый бордер, которые на тёмной теме
 * читались как контрол из чужого приложения.
 *
 * `--popup-*` синхронизируют выпадающий список: `--surface` в темах
 * synthos не существует, а именно на неё popup фолбэчился по умолчанию. */
Dropdown {
    background-color: var(--bg-search);
    color: var(--text);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 10px 14px;
    font-size: 14px;
    accent-color: var(--primary);
    --popup-background: var(--bg-shell);
    --popup-color: var(--text);
    --popup-border: var(--border);
    --popup-accent: var(--primary);
    --popup-hover-background: var(--surface-hover);
    --popup-hover-color: var(--text);
    --popup-selected-background: var(--primary-soft);
    --popup-selected-color: var(--primary);
    transition: border-color var(--duration-fast) var(--ease-standard);
}

Dropdown:hover {
    border-color: var(--primary);
}

/* DatePicker — поле и всплывающий календарь в теме приложения: без
 * правила виджет берёт свои дефолты (белая заливка поля и панели, серая
 * рамка), и в тёмной теме календарь выглядит чужим. `--cal-*` красят
 * панель: фон/рамка, приглушённые и выходные дни, наведение. */
DatePicker {
    background-color: var(--bg-search);
    color: var(--text);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    padding: 10px 14px;
    font-size: 14px;
    accent-color: var(--primary);
    --cal-panel-bg: var(--bg-shell);
    --cal-panel-border: var(--border);
    --cal-muted-color: var(--text-subtle);
    --cal-weekend-color: var(--text-muted);
    --cal-outside-color: var(--text-subtle);
    --cal-disabled-color: var(--border-strong);
    --cal-hover-bg: var(--surface-hover);
    --cal-today-color: var(--primary);
    --cal-selected-color: var(--on-primary);
    transition: border-color var(--duration-fast) var(--ease-standard);
}

DatePicker:hover {
    border-color: var(--primary);
}

/* TimePicker — поле и колонки часов/минут в теме приложения. Без правила
 * попап падает на белые умолчания виджета (см. time_picker.rs), как это
 * было у DatePicker. `--popup-*` красят список: фон и рамка попапа,
 * наведение и выбранная строка. */
TimePicker {
    background-color: var(--bg-search);
    color: var(--text);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    font-size: 14px;
    accent-color: var(--primary);
    --popup-background: var(--bg-shell);
    --popup-color: var(--text);
    --popup-border: var(--border);
    --popup-hover-background: var(--surface-hover);
    --popup-selected-background: var(--surface-selected);
    transition: border-color var(--duration-fast) var(--ease-standard);
}

TimePicker:hover {
    border-color: var(--primary);
}

/* Slider — тоже без глобального правила жил на дефолтах syngui: трек
 * #D1D5DB и заливка #3B82F6, то есть светлая полоса и синий «не из темы»
 * (см. slider.rs::build_display_list). Раскладка свойств там такая:
 *   background-color → незаполненный трек,
 *   color            → заполненная часть,
 *   accent-color     → заливка ползунка,
 *   border-color/-width → кольцо вокруг ползунка.
 * Кольцо красим в фон панели — так ползунок читается и на треке, и на
 * заливке, не сливаясь ни с тем, ни с другим. */
Slider {
    background-color: var(--border-strong);
    color: var(--primary);
    accent-color: var(--primary);
    border-color: var(--bg-panel);
    border-width: 2px;
}

/* MultilineTextEdit — глобально привязан к теме. Виджет сам рендерит фон
 * и рамку (background_color/border_color из ComputedStyle), поэтому
 * никаких DecoratedBox-обёрток сверху не нужно. `accent-color` идёт в
 * caret + selection. */
MultilineTextEdit {
    background-color: var(--bg-search);
    color: var(--text);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    accent-color: var(--primary);
    font-size: 14px;
    padding: 10px 14px;
    transition: border-color var(--duration-fast) var(--ease-standard);
}

MultilineTextEdit:focus {
    border-color: var(--primary);
}

/* CodeEditor — глобальный мост к теме. `--editor-*`/`--token-*` живут в
 * `:root` (см. theme_data::SynthosTheme::to_mss()); здесь они проецируются
 * в custom-MSS-поля виджета (`editor-bg`, `token-keyword`, …), которые
 * читает `CodeEditorPalette::from_style()` в syngui.
 *
 * Делаем глобально, чтобы любой CodeEditor в приложении (страница code,
 * редактор скила, будущие места) автоматически попадал в тему — точно
 * так же, как глобальный `TextField {}` выше. Локальные `.code-editor-mle`
 * / `.skill-code-editor` оставляем только для геометрии/font-family. */
CodeEditor {
    background-color:      var(--editor-bg);
    color:                 var(--editor-fg);
    font-family:           var(--code-editor-font-family);
    font-size:             var(--code-editor-font-size);

    editor-bg:             var(--editor-bg);
    editor-fg:             var(--editor-fg);
    editor-gutter-bg:      var(--editor-gutter-bg);
    editor-gutter-fg:      var(--editor-gutter-fg);
    editor-cursor:         var(--editor-cursor);
    editor-selection:      var(--editor-selection);
    editor-current-line:   var(--editor-current-line);
    editor-bracket-match:  var(--editor-bracket-match);
    editor-whitespace:     var(--editor-whitespace);

    token-keyword:         var(--token-keyword);
    token-keyword-control: var(--token-keyword-control);
    token-type:            var(--token-type);
    token-type-builtin:    var(--token-type-builtin);
    token-function:        var(--token-function);
    token-function-macro:  var(--token-function-macro);
    token-constant:        var(--token-constant);
    token-string:          var(--token-string);
    token-string-special:  var(--token-string-special);
    token-number:          var(--token-number);
    token-comment:         var(--token-comment);
    token-operator:        var(--token-operator);
    token-punctuation:     var(--token-punctuation);
    token-variable:        var(--token-variable);
    token-property:        var(--token-property);
    token-attribute:       var(--token-attribute);
    token-namespace:       var(--token-namespace);
    token-tag:             var(--token-tag);
}

/* PopupMenu — popup всех ContextMenu / Dropdown / overflow-menu в приложении.
 * Виджет читает `--popup-*` переменные через ComputedStyle (см.
 * syngui/widgets/overlay/menu.rs:574-578); привязываем их к токенам темы,
 * чтобы тёмные/светлые палитры красили popup согласованно.
 *
 * Сам shadow popup рисует внутри (push_shadow с blur=12, см. menu.rs:349) —
 * MSS управлять им сейчас нельзя, тень одинаковая во всех темах. */
/* ToolButton — иконочная кнопка без класса брала свои дефолты (#374151
 * глиф, #F3F4F6 подложка под курсором): в тёмной теме крестик тонул в
 * панели, а наведение вспыхивало почти белым. Фон только на :hover —
 * базовый background-color сделал бы «чип» под каждой иконкой в шапках. */
ToolButton {
    color: var(--text-muted);
    accent-color: var(--primary);
    border-radius: 8px;
}

ToolButton:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}

/* ColorPicker — поле в линию с остальными form-контролами, палитра — на
 * фоне панели через `--popup-*` (как у Dropdown). Без правила виджет
 * рисовал белый попап с тёмным текстом в любой теме. */
ColorPicker {
    background-color: var(--bg-search);
    color: var(--text);
    border-color: var(--border-soft);
    accent-color: var(--primary);
    border-radius: 10px;
    height: 32px;
    font-size: 13px;
    --popup-background: var(--bg-shell);
    --popup-color: var(--text);
    --popup-border: var(--border);
}

PopupMenu {
    background-color:            var(--bg-shell);
    color:                       var(--text);
    border-color:                var(--border);
    --popup-background:          var(--bg-shell);
    --popup-color:               var(--text);
    --popup-border:              var(--border);
    --popup-accent:              var(--primary);
    --popup-hover-background:    var(--surface-hover);
    --popup-hover-color:         var(--text);
    --popup-selected-background: var(--primary-soft);
    --popup-selected-color:      var(--primary);
}
