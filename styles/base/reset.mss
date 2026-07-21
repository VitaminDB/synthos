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
