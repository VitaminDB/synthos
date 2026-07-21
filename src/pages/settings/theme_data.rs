//! Встроенные темы synthos: 5 светлых + 6 тёмных.
//!
//! Каждая тема — это набор hex-значений для UI-токенов (фоны, accent,
//! текст, glass) **и** для палитры подсветки синтаксиса в редакторе кода
//! (`editor_*`, `token_*`). `to_mss()` выдаёт блок `:root { --var: ... }`,
//! который поверх базы `styles/base/variables.mss` применяется билдером
//! `AppBuilder::with_dynamic_theme(signal)`.
//!
//! Палитра редактора жёстко привязана к теме приложения: переменные
//! `--editor-*` и `--token-*` декларируются здесь, а виджет CodeEditor
//! читает их через мост в `styles/components/code_editor.mss`
//! (`editor-bg: var(--editor-bg)` и т. д.). Никакого отдельного `.theme-*`
//! класса на CodeEditor больше нет.

use std::fmt::Write;

pub struct SynthosTheme {
    pub id: &'static str,
    pub name: &'static str,
    pub is_dark: bool,

    pub bg_window: &'static str,
    pub bg_shell: &'static str,
    pub bg_rail: &'static str,
    pub bg_chats: &'static str,
    pub bg_chat: &'static str,
    pub bg_chat_dots: &'static str,
    pub bg_panel: &'static str,
    pub bg_search: &'static str,

    pub primary: &'static str,
    pub primary_hover: &'static str,
    pub primary_soft: &'static str,
    pub on_primary: &'static str,

    pub text: &'static str,
    pub text_muted: &'static str,
    pub text_subtle: &'static str,
    pub text_inverse: &'static str,

    pub surface_hover: &'static str,
    pub surface_selected: &'static str,
    pub border: &'static str,
    pub border_soft: &'static str,
    pub border_strong: &'static str,

    pub shadow_shell: &'static str,

    // Frosted-glass токены для оверлей-карточек (voice-панель и пр.).
    // Парсер MSS не раскрывает `var(...)` внутри `rgba()`, поэтому каждое
    // значение — готовый `rgba(...)` или `#RRGGBBAA` целиком.
    pub glass_card_bg: &'static str,
    pub glass_field_bg: &'static str,
    pub glass_border: &'static str,
    pub glass_shadow: &'static str,

    // ── CodeEditor — структурные цвета ───────────────────────────────
    pub editor_bg: &'static str,
    pub editor_fg: &'static str,
    pub editor_gutter_bg: &'static str,
    pub editor_gutter_fg: &'static str,
    pub editor_cursor: &'static str,
    pub editor_selection: &'static str,
    pub editor_current_line: &'static str,
    pub editor_bracket_match: &'static str,
    pub editor_whitespace: &'static str,

    // ── CodeEditor — токены подсветки синтаксиса ─────────────────────
    pub token_keyword: &'static str,
    pub token_keyword_control: &'static str,
    pub token_type: &'static str,
    pub token_type_builtin: &'static str,
    pub token_function: &'static str,
    pub token_function_macro: &'static str,
    pub token_constant: &'static str,
    pub token_string: &'static str,
    pub token_string_special: &'static str,
    pub token_number: &'static str,
    pub token_comment: &'static str,
    pub token_operator: &'static str,
    pub token_punctuation: &'static str,
    pub token_variable: &'static str,
    pub token_property: &'static str,
    pub token_attribute: &'static str,
    pub token_namespace: &'static str,
    pub token_tag: &'static str,
}

impl SynthosTheme {
    /// Превью — 5 выборочных цветов для карточки выбора темы.
    pub fn swatches(&self) -> [&'static str; 5] {
        [self.primary, self.bg_shell, self.bg_chat, self.bg_rail, self.border]
    }

    pub fn to_mss(&self) -> String {
        let mut s = String::with_capacity(2048);
        let _ = writeln!(s, ":root {{");
        let _ = writeln!(s, "    --bg-window:        {};", self.bg_window);
        let _ = writeln!(s, "    --bg-shell:         {};", self.bg_shell);
        let _ = writeln!(s, "    --bg-rail:          {};", self.bg_rail);
        let _ = writeln!(s, "    --bg-chats:         {};", self.bg_chats);
        let _ = writeln!(s, "    --bg-chat:          {};", self.bg_chat);
        let _ = writeln!(s, "    --bg-chat-dots:     {};", self.bg_chat_dots);
        let _ = writeln!(s, "    --bg-panel:         {};", self.bg_panel);
        let _ = writeln!(s, "    --bg-search:        {};", self.bg_search);
        let _ = writeln!(s, "    --primary:          {};", self.primary);
        let _ = writeln!(s, "    --primary-hover:    {};", self.primary_hover);
        let _ = writeln!(s, "    --primary-soft:     {};", self.primary_soft);
        let _ = writeln!(s, "    --on-primary:       {};", self.on_primary);
        let _ = writeln!(s, "    --text:             {};", self.text);
        let _ = writeln!(s, "    --text-muted:       {};", self.text_muted);
        let _ = writeln!(s, "    --text-subtle:      {};", self.text_subtle);
        let _ = writeln!(s, "    --text-inverse:     {};", self.text_inverse);
        let _ = writeln!(s, "    --surface-hover:    {};", self.surface_hover);
        let _ = writeln!(s, "    --surface-selected: {};", self.surface_selected);
        let _ = writeln!(s, "    --border:           {};", self.border);
        let _ = writeln!(s, "    --border-soft:      {};", self.border_soft);
        let _ = writeln!(s, "    --border-strong:    {};", self.border_strong);
        let _ = writeln!(s, "    --shadow-shell:     {};", self.shadow_shell);
        let _ = writeln!(s, "    --glass-card-bg:    {};", self.glass_card_bg);
        let _ = writeln!(s, "    --glass-field-bg:   {};", self.glass_field_bg);
        let _ = writeln!(s, "    --glass-border:     {};", self.glass_border);
        let _ = writeln!(s, "    --glass-shadow:     {};", self.glass_shadow);

        // CodeEditor — структурные токены.
        let _ = writeln!(s, "    --editor-bg:             {};", self.editor_bg);
        let _ = writeln!(s, "    --editor-fg:             {};", self.editor_fg);
        let _ = writeln!(s, "    --editor-gutter-bg:      {};", self.editor_gutter_bg);
        let _ = writeln!(s, "    --editor-gutter-fg:      {};", self.editor_gutter_fg);
        let _ = writeln!(s, "    --editor-cursor:         {};", self.editor_cursor);
        let _ = writeln!(s, "    --editor-selection:      {};", self.editor_selection);
        let _ = writeln!(s, "    --editor-current-line:   {};", self.editor_current_line);
        let _ = writeln!(s, "    --editor-bracket-match:  {};", self.editor_bracket_match);
        let _ = writeln!(s, "    --editor-whitespace:     {};", self.editor_whitespace);

        // CodeEditor — синтаксис-токены.
        let _ = writeln!(s, "    --token-keyword:         {};", self.token_keyword);
        let _ = writeln!(s, "    --token-keyword-control: {};", self.token_keyword_control);
        let _ = writeln!(s, "    --token-type:            {};", self.token_type);
        let _ = writeln!(s, "    --token-type-builtin:    {};", self.token_type_builtin);
        let _ = writeln!(s, "    --token-function:        {};", self.token_function);
        let _ = writeln!(s, "    --token-function-macro:  {};", self.token_function_macro);
        let _ = writeln!(s, "    --token-constant:        {};", self.token_constant);
        let _ = writeln!(s, "    --token-string:          {};", self.token_string);
        let _ = writeln!(s, "    --token-string-special:  {};", self.token_string_special);
        let _ = writeln!(s, "    --token-number:          {};", self.token_number);
        let _ = writeln!(s, "    --token-comment:         {};", self.token_comment);
        let _ = writeln!(s, "    --token-operator:        {};", self.token_operator);
        let _ = writeln!(s, "    --token-punctuation:     {};", self.token_punctuation);
        let _ = writeln!(s, "    --token-variable:        {};", self.token_variable);
        let _ = writeln!(s, "    --token-property:        {};", self.token_property);
        let _ = writeln!(s, "    --token-attribute:       {};", self.token_attribute);
        let _ = writeln!(s, "    --token-namespace:       {};", self.token_namespace);
        let _ = writeln!(s, "    --token-tag:             {};", self.token_tag);

        let _ = writeln!(s, "}}");
        s
    }
}

pub fn default_theme() -> SynthosTheme { coral_light() }

pub fn builtin_themes() -> Vec<SynthosTheme> {
    vec![
        coral_light(),
        ocean_light(),
        sand_light(),
        mint_light(),
        stone_light(),
        one_dark(),
        onyx_dark(),
        nord_dark(),
        dracula_dark(),
        midnight_dark(),
        monokai_dark(),
    ]
}

pub fn find(id: &str) -> Option<SynthosTheme> {
    builtin_themes().into_iter().find(|t| t.id == id)
}

// ─── Light ──────────────────────────────────────────────────────

fn coral_light() -> SynthosTheme {
    SynthosTheme {
        id: "coral_light", name: "Coral", is_dark: false,
        bg_window: "#EEECF2", bg_shell: "#FFFFFF", bg_rail: "#FAFAFC",
        bg_chats: "#FFFFFF", bg_chat: "#FFFFFF", bg_chat_dots: "#E6E6EE",
        bg_panel: "#FFFFFF", bg_search: "#F4F4F7",
        primary: "#EE5E48", primary_hover: "#E04A33",
        primary_soft: "#FFECE6", on_primary: "#FFFFFF",
        text: "#1C1D22", text_muted: "#6B7280", text_subtle: "#9CA3AF", text_inverse: "#FFFFFF",
        surface_hover: "#F3F3F5", surface_selected: "#FDF1EE",
        border: "#E5E7EB", border_soft: "#EEF0F3", border_strong: "#D6D8DE",
        shadow_shell: "0 28px 48px rgba(24, 24, 43, 0.10)",
        glass_card_bg: "rgba(255, 255, 255, 0.62)",
        glass_field_bg: "rgba(255, 255, 255, 0.50)",
        glass_border: "rgba(28, 30, 38, 0.10)",
        glass_shadow: "0 24px 64px rgba(20, 24, 40, 0.20)",

        // Editor — light (One Light в коралловом темпе).
        editor_bg: "#FFFFFF", editor_fg: "#1C1D22",
        editor_gutter_bg: "#F4F4F7", editor_gutter_fg: "#9CA3AF",
        editor_cursor: "#EE5E48", editor_selection: "#FBD7CD",
        editor_current_line: "#FAF5F4", editor_bracket_match: "#D6D8DE",
        editor_whitespace: "#E5E7EB",

        token_keyword: "#C13C2A", token_keyword_control: "#C13C2A",
        token_type: "#B0820F", token_type_builtin: "#B0820F",
        token_function: "#2862B0", token_function_macro: "#1F5394",
        token_constant: "#B85820", token_string: "#2A7C4F",
        token_string_special: "#1F5C3B", token_number: "#B85820",
        token_comment: "#9CA3AF", token_operator: "#4B5563",
        token_punctuation: "#6B7280", token_variable: "#1C1D22",
        token_property: "#2862B0", token_attribute: "#B0820F",
        token_namespace: "#B85820", token_tag: "#C13C2A",
    }
}

fn ocean_light() -> SynthosTheme {
    SynthosTheme {
        id: "ocean_light", name: "Ocean", is_dark: false,
        bg_window: "#E7EEF5", bg_shell: "#FFFFFF", bg_rail: "#F5F8FB",
        bg_chats: "#FFFFFF", bg_chat: "#FFFFFF", bg_chat_dots: "#D8E3EC",
        bg_panel: "#FFFFFF", bg_search: "#EEF4F8",
        primary: "#2E7DD1", primary_hover: "#1E62AE",
        primary_soft: "#DCEBFB", on_primary: "#FFFFFF",
        text: "#152231", text_muted: "#4E6479", text_subtle: "#89A0B3", text_inverse: "#FFFFFF",
        surface_hover: "#EEF3F8", surface_selected: "#DDEBFA",
        border: "#D3DCE5", border_soft: "#E6ECF2", border_strong: "#C0CBD6",
        shadow_shell: "0 28px 48px rgba(14, 40, 72, 0.12)",
        glass_card_bg: "rgba(252, 254, 255, 0.62)",
        glass_field_bg: "rgba(252, 254, 255, 0.50)",
        glass_border: "rgba(20, 50, 90, 0.10)",
        glass_shadow: "0 24px 64px rgba(14, 40, 72, 0.22)",

        editor_bg: "#FFFFFF", editor_fg: "#152231",
        editor_gutter_bg: "#EEF4F8", editor_gutter_fg: "#89A0B3",
        editor_cursor: "#2E7DD1", editor_selection: "#C5DDF4",
        editor_current_line: "#F4F8FC", editor_bracket_match: "#C0CBD6",
        editor_whitespace: "#D3DCE5",

        token_keyword: "#1E62AE", token_keyword_control: "#1E62AE",
        token_type: "#A86620", token_type_builtin: "#A86620",
        token_function: "#2E7DD1", token_function_macro: "#1E7CA8",
        token_constant: "#1F5394", token_string: "#2A7C4F",
        token_string_special: "#1F5C3B", token_number: "#B85820",
        token_comment: "#89A0B3", token_operator: "#4E6479",
        token_punctuation: "#6F8294", token_variable: "#152231",
        token_property: "#2E7DD1", token_attribute: "#A86620",
        token_namespace: "#B85820", token_tag: "#C13C2A",
    }
}

fn sand_light() -> SynthosTheme {
    SynthosTheme {
        id: "sand_light", name: "Sand", is_dark: false,
        bg_window: "#F3EEE5", bg_shell: "#FFFDF9", bg_rail: "#FBF6EC",
        bg_chats: "#FFFDF9", bg_chat: "#FFFDF9", bg_chat_dots: "#E9DEC9",
        bg_panel: "#FFFDF9", bg_search: "#F2EADA",
        primary: "#C97A2F", primary_hover: "#B5651C",
        primary_soft: "#FBE6CB", on_primary: "#FFFFFF",
        text: "#3A2E1D", text_muted: "#7A6A52", text_subtle: "#AE9E85", text_inverse: "#FFFFFF",
        surface_hover: "#F3ECDE", surface_selected: "#FBE8D2",
        border: "#E4D9C2", border_soft: "#EFE7D4", border_strong: "#D4C5A7",
        shadow_shell: "0 28px 48px rgba(90, 60, 20, 0.10)",
        glass_card_bg: "rgba(255, 252, 244, 0.62)",
        glass_field_bg: "rgba(255, 252, 244, 0.50)",
        glass_border: "rgba(90, 60, 20, 0.12)",
        glass_shadow: "0 24px 64px rgba(90, 60, 20, 0.22)",

        editor_bg: "#FFFDF9", editor_fg: "#3A2E1D",
        editor_gutter_bg: "#F2EADA", editor_gutter_fg: "#AE9E85",
        editor_cursor: "#C97A2F", editor_selection: "#EFD7B7",
        editor_current_line: "#F8F0DE", editor_bracket_match: "#D4C5A7",
        editor_whitespace: "#E4D9C2",

        token_keyword: "#B5651C", token_keyword_control: "#B5651C",
        token_type: "#876327", token_type_builtin: "#876327",
        token_function: "#1E62AE", token_function_macro: "#1E7CA8",
        token_constant: "#A85420", token_string: "#5C7A3D",
        token_string_special: "#41612C", token_number: "#A85420",
        token_comment: "#AE9E85", token_operator: "#7A6A52",
        token_punctuation: "#7A6A52", token_variable: "#3A2E1D",
        token_property: "#1E62AE", token_attribute: "#876327",
        token_namespace: "#A85420", token_tag: "#C13C2A",
    }
}

fn mint_light() -> SynthosTheme {
    SynthosTheme {
        id: "mint_light", name: "Mint", is_dark: false,
        bg_window: "#E7F1EC", bg_shell: "#FFFFFF", bg_rail: "#F3F9F6",
        bg_chats: "#FFFFFF", bg_chat: "#FFFFFF", bg_chat_dots: "#D5E7DD",
        bg_panel: "#FFFFFF", bg_search: "#EEF6F1",
        primary: "#199E77", primary_hover: "#128360",
        primary_soft: "#D5EFE5", on_primary: "#FFFFFF",
        text: "#132922", text_muted: "#466D5F", text_subtle: "#7E9C90", text_inverse: "#FFFFFF",
        surface_hover: "#EDF5F0", surface_selected: "#D7EEE4",
        border: "#D0E0D7", border_soft: "#E1EAE4", border_strong: "#B5CFC3",
        shadow_shell: "0 28px 48px rgba(10, 50, 38, 0.10)",
        glass_card_bg: "rgba(252, 255, 253, 0.62)",
        glass_field_bg: "rgba(252, 255, 253, 0.50)",
        glass_border: "rgba(10, 50, 38, 0.12)",
        glass_shadow: "0 24px 64px rgba(10, 50, 38, 0.20)",

        editor_bg: "#FFFFFF", editor_fg: "#132922",
        editor_gutter_bg: "#EEF6F1", editor_gutter_fg: "#7E9C90",
        editor_cursor: "#199E77", editor_selection: "#B8E1D0",
        editor_current_line: "#EFF7F3", editor_bracket_match: "#B5CFC3",
        editor_whitespace: "#D0E0D7",

        token_keyword: "#128360", token_keyword_control: "#128360",
        token_type: "#A86620", token_type_builtin: "#A86620",
        token_function: "#1E7CA8", token_function_macro: "#1F5394",
        token_constant: "#B85820", token_string: "#2A7C4F",
        token_string_special: "#1F5C3B", token_number: "#B85820",
        token_comment: "#7E9C90", token_operator: "#466D5F",
        token_punctuation: "#466D5F", token_variable: "#132922",
        token_property: "#1E7CA8", token_attribute: "#A86620",
        token_namespace: "#B85820", token_tag: "#C13C2A",
    }
}

fn stone_light() -> SynthosTheme {
    SynthosTheme {
        id: "stone_light", name: "Stone", is_dark: false,
        bg_window: "#E6E7EB", bg_shell: "#FDFDFD", bg_rail: "#F2F3F6",
        bg_chats: "#FDFDFD", bg_chat: "#FDFDFD", bg_chat_dots: "#D5D7DC",
        bg_panel: "#FDFDFD", bg_search: "#EDEEF2",
        primary: "#55616E", primary_hover: "#3E4A57",
        primary_soft: "#E1E5EA", on_primary: "#FFFFFF",
        text: "#1F242B", text_muted: "#58626F", text_subtle: "#8C95A2", text_inverse: "#FFFFFF",
        surface_hover: "#EAECF0", surface_selected: "#DCE0E7",
        border: "#D6D9DE", border_soft: "#E4E6EA", border_strong: "#B9BEC6",
        shadow_shell: "0 28px 48px rgba(20, 24, 32, 0.10)",
        glass_card_bg: "rgba(253, 253, 253, 0.62)",
        glass_field_bg: "rgba(253, 253, 253, 0.50)",
        glass_border: "rgba(20, 24, 32, 0.12)",
        glass_shadow: "0 24px 64px rgba(20, 24, 32, 0.20)",

        editor_bg: "#FDFDFD", editor_fg: "#1F242B",
        editor_gutter_bg: "#EDEEF2", editor_gutter_fg: "#8C95A2",
        editor_cursor: "#55616E", editor_selection: "#C7CCD3",
        editor_current_line: "#F4F5F7", editor_bracket_match: "#B9BEC6",
        editor_whitespace: "#D6D9DE",

        token_keyword: "#7C2D7E", token_keyword_control: "#7C2D7E",
        token_type: "#B0820F", token_type_builtin: "#B0820F",
        token_function: "#2862B0", token_function_macro: "#1E7CA8",
        token_constant: "#B85820", token_string: "#2A7C4F",
        token_string_special: "#1F5C3B", token_number: "#B85820",
        token_comment: "#8C95A2", token_operator: "#58626F",
        token_punctuation: "#58626F", token_variable: "#1F242B",
        token_property: "#2862B0", token_attribute: "#B0820F",
        token_namespace: "#B85820", token_tag: "#C13C2A",
    }
}

// ─── Dark ───────────────────────────────────────────────────────

/// Каноническая тема Atom One Dark — фон #282C34, accent #61AFEF.
/// Палитра подсветки — выверенный «классик», на который равняются
/// остальные тёмные темы.
fn one_dark() -> SynthosTheme {
    SynthosTheme {
        id: "one_dark", name: "One Dark", is_dark: true,
        bg_window: "#21252B", bg_shell: "#282C34", bg_rail: "#21252B",
        bg_chats: "#2C313A", bg_chat: "#282C34", bg_chat_dots: "#3B4048",
        bg_panel: "#2C313A", bg_search: "#21252B",
        primary: "#61AFEF", primary_hover: "#7BBFF5",
        primary_soft: "#1F2A3A", on_primary: "#0E1116",
        text: "#ABB2BF", text_muted: "#7F8794", text_subtle: "#5C6370", text_inverse: "#282C34",
        surface_hover: "#2C313A", surface_selected: "#1F2A3A",
        border: "#3B4048", border_soft: "#2C313A", border_strong: "#4B5263",
        shadow_shell: "0 32px 60px rgba(0, 0, 0, 0.50)",
        glass_card_bg: "rgba(40, 44, 52, 0.62)",
        glass_field_bg: "rgba(60, 66, 79, 0.55)",
        glass_border: "rgba(255, 255, 255, 0.08)",
        glass_shadow: "0 28px 64px rgba(0, 0, 0, 0.50)",

        editor_bg: "#282C34", editor_fg: "#ABB2BF",
        editor_gutter_bg: "#21252B", editor_gutter_fg: "#4B5263",
        editor_cursor: "#61AFEF", editor_selection: "#3E4451",
        editor_current_line: "#2C313C", editor_bracket_match: "#515A6B",
        editor_whitespace: "#3B4048",

        token_keyword: "#C678DD", token_keyword_control: "#C678DD",
        token_type: "#E5C07B", token_type_builtin: "#E5C07B",
        token_function: "#61AFEF", token_function_macro: "#56B6C2",
        token_constant: "#D19A66", token_string: "#98C379",
        token_string_special: "#56B6C2", token_number: "#D19A66",
        token_comment: "#5C6370", token_operator: "#ABB2BF",
        token_punctuation: "#ABB2BF", token_variable: "#ABB2BF",
        token_property: "#61AFEF", token_attribute: "#E5C07B",
        token_namespace: "#E5C07B", token_tag: "#E06C75",
    }
}

fn onyx_dark() -> SynthosTheme {
    SynthosTheme {
        id: "onyx_dark", name: "Onyx", is_dark: true,
        bg_window: "#0F1013", bg_shell: "#17191E", bg_rail: "#131418",
        bg_chats: "#181A1F", bg_chat: "#1B1D22", bg_chat_dots: "#2A2D34",
        bg_panel: "#181A1F", bg_search: "#21242A",
        primary: "#F2735A", primary_hover: "#FF8670",
        primary_soft: "#3A1E19", on_primary: "#FFFFFF",
        text: "#ECEEF2", text_muted: "#A5ABB5", text_subtle: "#6D7480", text_inverse: "#17191E",
        surface_hover: "#22252B", surface_selected: "#34201B",
        border: "#282B32", border_soft: "#222529", border_strong: "#363A42",
        shadow_shell: "0 32px 60px rgba(0, 0, 0, 0.55)",
        glass_card_bg: "rgba(20, 22, 28, 0.62)",
        glass_field_bg: "rgba(36, 40, 48, 0.55)",
        glass_border: "rgba(255, 255, 255, 0.08)",
        glass_shadow: "0 28px 64px rgba(0, 0, 0, 0.55)",

        editor_bg: "#1B1D22", editor_fg: "#ECEEF2",
        editor_gutter_bg: "#131418", editor_gutter_fg: "#6D7480",
        editor_cursor: "#F2735A", editor_selection: "#3A2C28",
        editor_current_line: "#22252B", editor_bracket_match: "#363A42",
        editor_whitespace: "#282B32",

        token_keyword: "#F2735A", token_keyword_control: "#F2735A",
        token_type: "#E0B341", token_type_builtin: "#E0B341",
        token_function: "#7AC5E0", token_function_macro: "#56B6C2",
        token_constant: "#FFB454", token_string: "#A1C781",
        token_string_special: "#56B6C2", token_number: "#FFB454",
        token_comment: "#6D7480", token_operator: "#ECEEF2",
        token_punctuation: "#A5ABB5", token_variable: "#ECEEF2",
        token_property: "#7AC5E0", token_attribute: "#E0B341",
        token_namespace: "#FFB454", token_tag: "#E06C75",
    }
}

fn nord_dark() -> SynthosTheme {
    SynthosTheme {
        id: "nord_dark", name: "Nord", is_dark: true,
        bg_window: "#1A2030", bg_shell: "#242B3B", bg_rail: "#1F2533",
        bg_chats: "#252C3D", bg_chat: "#2B334A", bg_chat_dots: "#3A4360",
        bg_panel: "#252C3D", bg_search: "#2F3852",
        primary: "#88C0D0", primary_hover: "#A6D2DF",
        primary_soft: "#2A3A52", on_primary: "#0C1320",
        text: "#E7EEF5", text_muted: "#A1B0C5", text_subtle: "#6F809A", text_inverse: "#1A2030",
        surface_hover: "#2F3750", surface_selected: "#344666",
        border: "#313A52", border_soft: "#2A3246", border_strong: "#45506D",
        shadow_shell: "0 32px 60px rgba(0, 0, 0, 0.45)",
        glass_card_bg: "rgba(28, 36, 52, 0.62)",
        glass_field_bg: "rgba(44, 54, 78, 0.55)",
        glass_border: "rgba(255, 255, 255, 0.08)",
        glass_shadow: "0 28px 64px rgba(0, 0, 0, 0.50)",

        editor_bg: "#2B334A", editor_fg: "#E7EEF5",
        editor_gutter_bg: "#1F2533", editor_gutter_fg: "#6F809A",
        editor_cursor: "#88C0D0", editor_selection: "#3D4D6B",
        editor_current_line: "#313A52", editor_bracket_match: "#45506D",
        editor_whitespace: "#3A4360",

        token_keyword: "#81A1C1", token_keyword_control: "#81A1C1",
        token_type: "#EBCB8B", token_type_builtin: "#EBCB8B",
        token_function: "#88C0D0", token_function_macro: "#8FBCBB",
        token_constant: "#B48EAD", token_string: "#A3BE8C",
        token_string_special: "#8FBCBB", token_number: "#D08770",
        token_comment: "#6F809A", token_operator: "#ECEFF4",
        token_punctuation: "#D8DEE9", token_variable: "#E7EEF5",
        token_property: "#88C0D0", token_attribute: "#EBCB8B",
        token_namespace: "#D08770", token_tag: "#BF616A",
    }
}

fn dracula_dark() -> SynthosTheme {
    SynthosTheme {
        id: "dracula_dark", name: "Dracula", is_dark: true,
        bg_window: "#161722", bg_shell: "#1E1F2C", bg_rail: "#191A25",
        bg_chats: "#20212E", bg_chat: "#232435", bg_chat_dots: "#2F2F46",
        bg_panel: "#20212E", bg_search: "#2B2C3E",
        primary: "#FF79C6", primary_hover: "#FF94D3",
        primary_soft: "#3A1F33", on_primary: "#18121C",
        text: "#F2F2FB", text_muted: "#A6A6C8", text_subtle: "#6E6E93", text_inverse: "#1E1F2C",
        surface_hover: "#2B2C40", surface_selected: "#3A2338",
        border: "#2E2F46", border_soft: "#24243A", border_strong: "#3C3E5A",
        shadow_shell: "0 32px 60px rgba(0, 0, 0, 0.55)",
        glass_card_bg: "rgba(24, 26, 42, 0.62)",
        glass_field_bg: "rgba(40, 42, 60, 0.55)",
        glass_border: "rgba(255, 255, 255, 0.08)",
        glass_shadow: "0 28px 64px rgba(0, 0, 0, 0.55)",

        editor_bg: "#232435", editor_fg: "#F2F2FB",
        editor_gutter_bg: "#191A25", editor_gutter_fg: "#6E6E93",
        editor_cursor: "#FF79C6", editor_selection: "#44475A",
        editor_current_line: "#2D2E42", editor_bracket_match: "#3C3E5A",
        editor_whitespace: "#2E2F46",

        token_keyword: "#FF79C6", token_keyword_control: "#FF79C6",
        token_type: "#8BE9FD", token_type_builtin: "#8BE9FD",
        token_function: "#50FA7B", token_function_macro: "#50FA7B",
        token_constant: "#BD93F9", token_string: "#F1FA8C",
        token_string_special: "#FFB86C", token_number: "#BD93F9",
        token_comment: "#6272A4", token_operator: "#F2F2FB",
        token_punctuation: "#F8F8F2", token_variable: "#F2F2FB",
        token_property: "#50FA7B", token_attribute: "#FFB86C",
        token_namespace: "#BD93F9", token_tag: "#FF79C6",
    }
}

fn midnight_dark() -> SynthosTheme {
    SynthosTheme {
        id: "midnight_dark", name: "Midnight", is_dark: true,
        bg_window: "#0D1320", bg_shell: "#141B2B", bg_rail: "#0F1624",
        bg_chats: "#171F31", bg_chat: "#1C2538", bg_chat_dots: "#28324A",
        bg_panel: "#171F31", bg_search: "#202B42",
        primary: "#4DA3FF", primary_hover: "#6EB4FF",
        primary_soft: "#17304F", on_primary: "#081422",
        text: "#EAF0FA", text_muted: "#9DAFC9", text_subtle: "#63779A", text_inverse: "#141B2B",
        surface_hover: "#1D2943", surface_selected: "#1E3B5A",
        border: "#213252", border_soft: "#1A243B", border_strong: "#334872",
        shadow_shell: "0 32px 60px rgba(0, 0, 0, 0.50)",
        glass_card_bg: "rgba(20, 30, 52, 0.62)",
        glass_field_bg: "rgba(34, 46, 72, 0.55)",
        glass_border: "rgba(255, 255, 255, 0.08)",
        glass_shadow: "0 28px 64px rgba(0, 0, 0, 0.50)",

        editor_bg: "#1C2538", editor_fg: "#EAF0FA",
        editor_gutter_bg: "#0F1624", editor_gutter_fg: "#63779A",
        editor_cursor: "#4DA3FF", editor_selection: "#1F3F66",
        editor_current_line: "#1F2A40", editor_bracket_match: "#334872",
        editor_whitespace: "#28324A",

        token_keyword: "#6EB4FF", token_keyword_control: "#6EB4FF",
        token_type: "#FFCB6B", token_type_builtin: "#FFCB6B",
        token_function: "#82AAFF", token_function_macro: "#89DDFF",
        token_constant: "#C792EA", token_string: "#A0E07F",
        token_string_special: "#89DDFF", token_number: "#F78C6C",
        token_comment: "#63779A", token_operator: "#EAF0FA",
        token_punctuation: "#B1BFD3", token_variable: "#EAF0FA",
        token_property: "#82AAFF", token_attribute: "#FFCB6B",
        token_namespace: "#F78C6C", token_tag: "#FF6B9C",
    }
}

fn monokai_dark() -> SynthosTheme {
    SynthosTheme {
        id: "monokai_dark", name: "Monokai", is_dark: true,
        bg_window: "#1D1E18", bg_shell: "#272822", bg_rail: "#1F201A",
        bg_chats: "#2B2C24", bg_chat: "#323328", bg_chat_dots: "#45463A",
        bg_panel: "#2B2C24", bg_search: "#393A2E",
        primary: "#F9A825", primary_hover: "#FFBD4A",
        primary_soft: "#3D3019", on_primary: "#1F200D",
        text: "#F8F8F2", text_muted: "#CFCFC2", text_subtle: "#75715E", text_inverse: "#272822",
        surface_hover: "#34352A", surface_selected: "#45361C",
        border: "#3C3D31", border_soft: "#32332A", border_strong: "#545545",
        shadow_shell: "0 32px 60px rgba(0, 0, 0, 0.55)",
        glass_card_bg: "rgba(34, 36, 28, 0.62)",
        glass_field_bg: "rgba(48, 50, 38, 0.55)",
        glass_border: "rgba(255, 255, 255, 0.08)",
        glass_shadow: "0 28px 64px rgba(0, 0, 0, 0.55)",

        editor_bg: "#272822", editor_fg: "#F8F8F2",
        editor_gutter_bg: "#1F201A", editor_gutter_fg: "#75715E",
        editor_cursor: "#F9A825", editor_selection: "#49483E",
        editor_current_line: "#34352A", editor_bracket_match: "#545545",
        editor_whitespace: "#3C3D31",

        token_keyword: "#F92672", token_keyword_control: "#F92672",
        token_type: "#66D9EF", token_type_builtin: "#66D9EF",
        token_function: "#A6E22E", token_function_macro: "#A6E22E",
        token_constant: "#AE81FF", token_string: "#E6DB74",
        token_string_special: "#FD971F", token_number: "#AE81FF",
        token_comment: "#75715E", token_operator: "#F8F8F2",
        token_punctuation: "#F8F8F2", token_variable: "#F8F8F2",
        token_property: "#A6E22E", token_attribute: "#FD971F",
        token_namespace: "#FD971F", token_tag: "#F92672",
    }
}
