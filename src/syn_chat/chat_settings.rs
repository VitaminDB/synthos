//! Настройки Syn-чата, которые хранятся в его файле рядом с лентой:
//! инструменты, пул `autotools`, скилы и системный промпт. Сэмплинг и
//! размышления лежат там же, но отдельным полем (`StoredChat.syn_params`,
//! появилось раньше).
//!
//! Сигналы панелей общие на приложение (`AppCtx.tools`,
//! `AppCtx.skills_active`, `SynChatCtx.system_prompt` / `prompt_active`) и
//! показывают открытый чат: открытие чата выставляет их из файла
//! ([`ChatSettings::apply`]), автосейв снимает их обратно
//! ([`ChatSettings::capture`]).
//!
//! Ход переживает переключение чатов, поэтому инструменты, вызванные посреди
//! хода (`autotools`, `subagent`, автокомпакт), берут настройки не из
//! панелей, а из снимка хода — [`for_turn`].

use serde::{Deserialize, Serialize};
use syngui::prelude::*;

use crate::context::AppCtx;
use crate::syn_chat::params::SamplingParams;
use crate::syn_chat::SynChatCtx;

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatSettings {
    /// Инструменты, чьи схемы уходят модели (`AppCtx.tools.active`).
    pub tools_active: Vec<String>,
    /// Пул `autotools` (`AppCtx.tools.auto`).
    pub tools_auto: Vec<String>,
    /// Активные скилы: `autoskill` предлагает модели только их, а если ни
    /// одного нет — все (см. [`offered_skills`]).
    pub skills_active: Vec<String>,
    /// Текст системного промпта. Свой у каждого чата: правка в одном чате
    /// не трогает ни другие чаты, ни библиотеку пресетов.
    pub system_prompt: String,
    /// id пресета библиотеки, из которого взят текст. Пусто или пресет
    /// удалён — текст чата ни к одному пресету не привязан.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub prompt_preset: String,
}

impl ChatSettings {
    /// Настройки из сигналов панелей — то, что сейчас видно у открытого
    /// чата. Без `AppCtx` (юнит-тесты чата) списки пустые.
    pub fn capture(ctx: &SynChatCtx) -> Self {
        let app = try_use_context::<AppCtx>();
        let list = |f: fn(&AppCtx) -> RwSignal<Vec<String>>| {
            app.as_ref().map(|a| f(a).get_untracked()).unwrap_or_default()
        };
        Self {
            tools_active: list(|a| a.tools.active),
            tools_auto: list(|a| a.tools.auto),
            skills_active: list(|a| a.skills_active),
            system_prompt: ctx.system_prompt.get_untracked(),
            prompt_preset: ctx.prompt_active.get_untracked(),
        }
    }

    /// Выставить настройки в сигналы панелей. Сигнал, чьё значение не
    /// меняется, не трогается: иначе переключение между чатами с одинаковым
    /// набором пересобирало бы секции панелей.
    pub fn apply(&self, ctx: &SynChatCtx) {
        if let Some(app) = try_use_context::<AppCtx>() {
            set_changed(app.tools.active, &self.tools_active);
            set_changed(app.tools.auto, &self.tools_auto);
            set_changed(app.skills_active, &self.skills_active);
        }
        set_changed(ctx.system_prompt, &self.system_prompt);
        set_changed(ctx.prompt_active, &self.prompt_preset);
    }

    /// Подписать текущий эффект на все сигналы настроек — автосейв узнаёт
    /// о правке любой из них.
    pub fn track(ctx: &SynChatCtx) {
        if let Some(app) = try_use_context::<AppCtx>() {
            app.tools.active.with(|_| ());
            app.tools.auto.with(|_| ());
            app.skills_active.with(|_| ());
        }
        ctx.system_prompt.with(|_| ());
        ctx.prompt_active.with(|_| ());
    }
}

fn set_changed<T: Clone + PartialEq + Send + Sync + 'static>(signal: RwSignal<T>, value: &T) {
    if signal.with_untracked(|v| v != value) {
        signal.set(value.clone());
    }
}

/// Скилы, которые `autoskill` предлагает модели: активные в чате, а если
/// среди них нет ни одного существующего — все. Порядок — как в библиотеке.
pub fn offered_skills(all: &[crate::skills::Skill], active: &[String]) -> Vec<crate::skills::Skill> {
    let picked: Vec<crate::skills::Skill> = all
        .iter()
        .filter(|s| active.contains(&s.id))
        .cloned()
        .collect();
    if picked.is_empty() {
        all.to_vec()
    } else {
        picked
    }
}

/// Всё, с чем идёт ход: настройки чата и его сэмплинг (сырые, до пресета
/// модели).
#[derive(Debug, Clone, PartialEq)]
pub struct TurnSettings {
    pub chat: ChatSettings,
    pub params: SamplingParams,
}

impl TurnSettings {
    /// Настройки открытого чата — с ними начинается его ход.
    pub fn capture(ctx: &SynChatCtx) -> Self {
        Self {
            chat: ChatSettings::capture(ctx),
            params: ctx.params.get_untracked(),
        }
    }
}

/// Настройки идущего хода. Ход мог уйти в фон, и панели показывают уже
/// другой чат, — тогда берётся снимок, снятый на старте хода. Хода нет —
/// настройки открытого чата.
pub fn for_turn(ctx: &SynChatCtx) -> TurnSettings {
    if ctx.generating_chat.get_untracked().is_some() {
        if let Some(t) = ctx.turn_settings.get_untracked() {
            return t;
        }
    }
    TurnSettings::capture(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::Skill;

    fn skill(id: &str) -> Skill {
        Skill {
            id: id.into(),
            name: id.to_uppercase(),
            description: String::new(),
            content: String::new(),
        }
    }

    #[test]
    fn offered_skills_follow_active_or_fall_back_to_all() {
        let all = vec![skill("a"), skill("b"), skill("c")];
        let ids = |v: Vec<Skill>| v.into_iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(ids(offered_skills(&all, &[])), ["a", "b", "c"]);
        // Порядок — библиотеки, а не порядок включения.
        assert_eq!(ids(offered_skills(&all, &["c".into(), "a".into()])), ["a", "c"]);
        // Активный скил удалён из библиотеки — как будто активных нет.
        assert_eq!(ids(offered_skills(&all, &["gone".into()])), ["a", "b", "c"]);
    }

    #[test]
    fn old_chat_files_have_no_settings_and_empty_preset_is_not_written() {
        let s = ChatSettings {
            tools_active: vec!["bash".into()],
            system_prompt: "Ты — помощник".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("prompt_preset"), "{json}");
        assert_eq!(serde_json::from_str::<ChatSettings>(&json).unwrap(), s);
        // Файл без части полей (или записанный будущей версией) читается.
        let partial: ChatSettings = serde_json::from_str(r#"{"skills_active":["x"],"new":1}"#).unwrap();
        assert_eq!(partial.skills_active, ["x"]);
        assert!(partial.tools_active.is_empty());
    }
}
