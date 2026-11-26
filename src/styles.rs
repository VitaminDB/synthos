//! MSS stylesheets are split into per-component files and concatenated at
//! compile time. Keeps one component = one file and lets variables from
//! `base/variables.mss` resolve in every other block.

pub fn styles() -> &'static str {
    STYLES
}

const STYLES: &str = concat!(
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
        include_str!("../styles/components/conversation_item.mss"),
        "\n",
        include_str!("../styles/components/chat_list.mss"),
        "\n",
        include_str!("../styles/components/search.mss"),
        "\n",
        include_str!("../styles/components/panel_header.mss"),
        "\n",
        include_str!("../styles/components/workspace_frame.mss"),
        "\n",
        include_str!("../styles/components/message_area.mss"),
        "\n",
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
        "\n",
        "\n",
        include_str!("../styles/components/tool_messages.mss"),
        "\n",
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
        include_str!("../styles/components/syn_chat.mss"),
        "\n",
);

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
