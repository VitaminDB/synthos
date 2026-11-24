//! MSS stylesheets are split into per-component files and concatenated at
//! compile time. Keeps one component = one file and lets variables from
//! `base/variables.mss` resolve in every other block.

/// Вся таблица: свои стили, стили редактора узлов (`syn_node_editor::STYLES`)
/// на прежнем месте каскада, затем остальные свои.
pub fn styles() -> &'static str {
    static ALL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ALL.get_or_init(|| [HEAD, syn_node_editor::STYLES, TAIL].concat())
}

/// До стилей редактора узлов.
const HEAD: &str = {
    concat!(
        include_str!("../styles/base/variables.mss"),
        "\n",
        include_str!("../styles/base/reset.mss"),
        "\n",
        include_str!("../styles/base/layout.mss"),
        "\n",
        include_str!("../styles/components/scrollbars.mss"),
        "\n",
        include_str!("../styles/components/tab_bar_global.mss"),
        "\n",
        include_str!("../styles/components/notification.mss"),
        "\n",
        include_str!("../styles/layout/shell.mss"),
        "\n",
        include_str!("../styles/components/nav_rail.mss"),
        "\n",
        "\n",
        include_str!("../styles/components/conversation_item.mss"),
        "\n",
        include_str!("../styles/components/chat_list.mss"),
        "\n",
        include_str!("../styles/components/search.mss"),
        "\n",
        include_str!("../styles/components/panel_header.mss"),
        include_str!("../styles/components/workspace_frame.mss"),
        "\n",
        include_str!("../styles/components/message_area.mss"),
        include_str!("../styles/components/chat_float_window.mss"),
        "\n",
        include_str!("../styles/components/message_bubble.mss"),
        "\n",
        include_str!("../styles/components/date_divider.mss"),
        "\n",
        include_str!("../styles/components/compaction_marker.mss"),
        "\n",
        include_str!("../styles/components/attachments_strip.mss"),
        "\n",
        include_str!("../styles/components/media_viewer.mss"),
        "\n",
        include_str!("../styles/components/input_panel.mss"),
        "\n",
        include_str!("../styles/components/kb_chip.mss"),
        "\n",
        include_str!("../styles/components/voice_fab.mss"),
        "\n",
        include_str!("../styles/components/voice_panel.mss"),
        "\n",
        include_str!("../styles/components/right_panel.mss"),
        "\n",
        include_str!("../styles/components/details.mss"),
        "\n",
        include_str!("../styles/layout/settings.mss"),
        "\n",
        include_str!("../styles/components/settings_sidebar.mss"),
        "\n",
        include_str!("../styles/components/settings_form.mss"),
        "\n",
        include_str!("../styles/components/settings_about.mss"),
        "\n",
        include_str!("../styles/components/settings_themes.mss"),
        "\n",
        include_str!("../styles/components/settings_skills.mss"),
        "\n",
        include_str!("../styles/components/settings_audio_models.mss"),
        "\n",
        include_str!("../styles/components/settings_kb.mss"),
        include_str!("../styles/components/training.mss"),
        "\n",
        include_str!("../styles/components/tool_messages.mss"),
        include_str!("../styles/components/wizard_panel.mss"),
        "\n",
        include_str!("../styles/components/tool_group.mss"),
        "\n",
        include_str!("../styles/components/code_editor.mss"),
        "\n",
        include_str!("../styles/components/code_editor_dialogs.mss"),
        "\n",
        include_str!("../styles/components/code_editor_terminal.mss"),
        "\n",
        include_str!("../styles/components/terminal_tabs.mss"),
        "\n",
    )
};

/// После стилей редактора узлов.
const TAIL: &str = {
    concat!(
        "\n",
        include_str!("../styles/components/node_editor_models_panel.mss"),
        "\n",
        include_str!("../styles/components/template_picker.mss"),
        "\n",
        include_str!("../styles/components/syn_explorer.mss"),
        "\n",
        include_str!("../styles/components/syn_chat.mss"),
        "\n",
        include_str!("../../synshell/crates/syn-notes/styles/notes.mss"),
        "\n",
        include_str!("../styles/layout/huggingface.mss"),
        "\n",
        include_str!("../styles/components/hf_header.mss"),
        "\n",
        include_str!("../styles/components/hf_list.mss"),
        "\n",
        include_str!("../styles/components/hf_detail.mss"),
        include_str!("../styles/components/hf_dock.mss"),
        "\n",
        include_str!("../styles/components/hf_dialog.mss"),
    )
};

#[cfg(test)]
mod tests {
    /// Один сломанный селектор роняет весь stylesheet в рантайме, а не в
    /// сборке — держим парсинг под тестом.
    #[test]
    fn stylesheet_parses() {
        syngui::mss::parse_stylesheet_str(super::styles())
            .expect("styles.mss должен парситься");
    }
}
