//! Модель строк ленты чата: из `Vec<ChatMsg>` и состояния UI получается
//! список строк, у каждой — стабильный ключ и версия.
//!
//! Зачем. Лента сейчас пересобирается целиком на каждое изменение
//! `messages` (в агентном ходе — на каждый tool_call и tool_result), потому
//! что сверка детей в syngui позиционная: при другой длине список детей
//! пересоздаётся вместе со всеми пузырьками. Ключ опознаёт строку между
//! пересборками, а версия отвечает на вопрос «надо ли собирать её заново»:
//! если версия та же, виджет строки можно не трогать.
//!
//! Единица строки — не сообщение: одиночное сообщение, свёрнутая группа
//! подряд идущих tool-вызовов (`minimal`-режим), маркер компактификации со
//! своими свёрнутыми сообщениями, разделитель даты и сообщения очереди
//! отправки.
//!
//! В версию входит всё, от чего зависит вид строки, включая то, что её
//! сборщик читает без подписки: `pipeline_tail` и `wizard_answered`
//! выводятся из длины ленты. Сами флаги, а не длина: иначе каждое новое
//! сообщение обесценивало бы все строки разом.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use crate::syn_chat::state::{ChatMsg, ChatMsgKind, ChatMsgRole, QueuedMsg, WizardDraft};

/// Строка ленты.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneRow {
    /// Опознаёт строку между пересборками. Переживает вставку и удаление
    /// соседей: у сообщений это [`crate::agent::state::UiId`], у очереди —
    /// её собственный id.
    pub key: u64,
    /// Меняется ровно тогда, когда строку надо собрать заново.
    pub version: u64,
    pub kind: LaneKind,
}

/// Что показывает строка.
#[derive(Debug, Clone, PartialEq)]
pub enum LaneKind {
    /// Разделитель с датой в самом верху ленты.
    DateDivider,
    /// Одиночное сообщение (в том числе tool-карточка вне группы).
    Message(MessageRow),
    /// Свёрнутая группа одинаковых tool-вызовов (`minimal`-режим).
    Group(GroupRow),
    /// Маркер компактификации: под ним прячутся свёрнутые сообщения.
    Marker(MarkerRow),
    /// Сообщение из очереди отправки — хвост ленты.
    Queued(QueuedRow),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageRow {
    /// Индекс в `messages`: по нему работают session API и карты
    /// раскрытия, поэтому он входит в версию.
    pub idx: usize,
    /// Последняя реплика ассистента: к ней привязаны стрим-хвост и regen.
    pub is_last_assistant: bool,
    /// Пустой плейсхолдер ассистента во время генерации — «Печатает…».
    pub is_typing: bool,
    /// Подсвечено переходом из глобального поиска.
    pub highlighted: bool,
    /// Открыта правка сообщения на месте.
    pub editing: bool,
    /// Карточка `pipelines` — хвост ленты: под ней живой статус прогона.
    pub pipeline_tail: bool,
    /// Панель визарда уже отвечена (разговор ушёл дальше).
    pub wizard_answered: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupRow {
    /// Индекс первого сообщения группы — ключ карты раскрытия.
    pub start_idx: usize,
    /// Число пар (вызов, результат); минимум 2.
    pub count: usize,
    pub tool_name: String,
    /// Сколько результатов в группе — с ошибкой.
    pub err_count: usize,
    /// Индексы сообщений группы, длина — `count * 2`.
    pub items: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarkerRow {
    pub idx: usize,
    pub iteration: u32,
    /// Индексы сообщений, свёрнутых этой итерацией.
    pub compacted: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueuedRow {
    pub id: u64,
    /// Сообщение очереди открыто на правку.
    pub editing: bool,
}

/// Состояние UI, от которого зависят строки. Ссылки, а не копии: модель
/// строится на каждое изменение ленты, лишние клоны здесь ни к чему.
pub struct LaneUi<'a> {
    pub tool_mode: &'a str,
    /// Идёт генерация: влияет только на последнюю строку.
    pub pending: bool,
    pub highlight: Option<usize>,
    pub editing: Option<usize>,
    /// Ключ темы: от неё зависит подсветка кода внутри пузырьков.
    pub theme_key: u64,
    pub thinking_open: &'a HashMap<usize, bool>,
    pub tool_body_open: &'a HashMap<usize, bool>,
    pub tool_group_open: &'a HashMap<usize, bool>,
    pub compaction_open: &'a HashMap<u32, bool>,
    pub wizard_drafts: &'a HashMap<usize, WizardDraft>,
    /// Сообщение очереди, открытое на правку.
    pub queue_editing: Option<u64>,
}

const TAG_DIVIDER: u64 = 1;
const TAG_MESSAGE: u64 = 2;
const TAG_GROUP: u64 = 3;
const TAG_MARKER: u64 = 4;
const TAG_QUEUED: u64 = 5;

fn hasher() -> std::collections::hash_map::DefaultHasher {
    std::collections::hash_map::DefaultHasher::new()
}

fn mix(tag: u64, value: u64) -> u64 {
    let mut h = hasher();
    tag.hash(&mut h);
    value.hash(&mut h);
    h.finish()
}

/// Ключ строки сообщения — тот же, что кладёт [`build`]. Нужен ленте,
/// чтобы прокрутиться к сообщению (переход из глобального поиска).
pub fn message_key(msg: &ChatMsg) -> u64 {
    mix(TAG_MESSAGE, msg.ui_id.0)
}

/// Снимок содержимого сообщения. `ui_id` в хэш не входит (его `Hash`
/// вырожден), поэтому одинаковые по содержимому сообщения дают одну версию.
fn content_hash(msg: &ChatMsg) -> u64 {
    let mut h = hasher();
    msg.hash(&mut h);
    h.finish()
}

fn draft_hash(h: &mut impl Hasher, draft: Option<&WizardDraft>) {
    match draft {
        None => 0u8.hash(h),
        Some(d) => {
            1u8.hash(h);
            d.selected.hash(h);
            d.custom.hash(h);
            d.custom_open.hash(h);
            d.deadline.hash(h);
            d.expired.hash(h);
            d.dismissed.hash(h);
        }
    }
}

/// Строки ленты по сообщениям, очереди и состоянию UI.
///
/// Свёрнутые компактификацией сообщения в ленту не идут: они рисуются
/// внутри своего маркера.
pub fn build(msgs: &[ChatMsg], queued: &[QueuedMsg], ui: &LaneUi) -> Vec<LaneRow> {
    let mut rows: Vec<LaneRow> = Vec::with_capacity(msgs.len() + queued.len() + 1);
    let mut used: HashSet<u64> = HashSet::with_capacity(msgs.len() + queued.len() + 1);
    let mut push = |rows: &mut Vec<LaneRow>, key: u64, version: u64, kind: LaneKind| {
        // Ключи должны быть уникальны: сообщение могли клонировать вместе с
        // его номером. Дубликат разводим номером повторения.
        let mut key = key;
        let mut bump = 0u64;
        while !used.insert(key) {
            bump += 1;
            key = mix(key, bump);
        }
        rows.push(LaneRow { key, version, kind });
    };

    push(&mut rows, TAG_DIVIDER, TAG_DIVIDER, LaneKind::DateDivider);

    // Видимые сообщения и их исходные индексы: группировка идёт по видимым,
    // а session API и карты раскрытия работают с исходными.
    let mut visible: Vec<usize> = Vec::with_capacity(msgs.len());
    for (i, m) in msgs.iter().enumerate() {
        if m.compacted_iter.is_none() {
            visible.push(i);
        }
    }
    let last_vis = visible.len().saturating_sub(1);

    for entry in group_lane(msgs, &visible, ui.tool_mode) {
        match entry {
            Entry::Single(vi) => {
                let idx = visible[vi];
                let msg = &msgs[idx];
                if let ChatMsgKind::CompactionMarker { iteration, .. } = &msg.kind {
                    let iteration = *iteration;
                    let compacted: Vec<usize> = msgs
                        .iter()
                        .enumerate()
                        .filter(|(_, m)| m.compacted_iter == Some(iteration))
                        .map(|(i, _)| i)
                        .collect();
                    let open = ui.compaction_open.get(&iteration).copied().unwrap_or(false);
                    let mut h = hasher();
                    TAG_MARKER.hash(&mut h);
                    idx.hash(&mut h);
                    content_hash(msg).hash(&mut h);
                    for i in &compacted {
                        content_hash(&msgs[*i]).hash(&mut h);
                        ui.tool_body_open.get(i).copied().unwrap_or(false).hash(&mut h);
                        ui.thinking_open.get(i).copied().unwrap_or(false).hash(&mut h);
                    }
                    open.hash(&mut h);
                    ui.tool_mode.hash(&mut h);
                    ui.theme_key.hash(&mut h);
                    push(
                        &mut rows,
                        mix(TAG_MARKER, msg.ui_id.0),
                        h.finish(),
                        LaneKind::Marker(MarkerRow { idx, iteration, compacted }),
                    );
                    continue;
                }

                let is_last_assistant = vi == last_vis
                    && msg.role == ChatMsgRole::Assistant
                    && matches!(msg.kind, ChatMsgKind::Text);
                let is_typing = ui.pending
                    && vi == last_vis
                    && msg.role == ChatMsgRole::Assistant
                    && msg.body.is_empty();
                // Флаги из длины ленты ставим только тем карточкам, которые
                // их читают: живой прогон — под хвостовым вызовом
                // `pipelines`, панель визарда — пока разговор не ушёл
                // дальше. Иначе каждое новое сообщение меняло бы версию
                // всех строк и мемоизация не работала бы.
                let tool = match &msg.kind {
                    ChatMsgKind::ToolCall { tool_name } => tool_name.as_str(),
                    _ => "",
                };
                let row = MessageRow {
                    idx,
                    is_last_assistant,
                    is_typing,
                    highlighted: ui.highlight == Some(idx),
                    editing: ui.editing == Some(idx),
                    pipeline_tail: tool == "pipelines" && idx + 1 == msgs.len(),
                    wizard_answered: tool == "wizard" && msgs.len() > idx + 2,
                };
                let mut h = hasher();
                TAG_MESSAGE.hash(&mut h);
                content_hash(msg).hash(&mut h);
                row.hash_into(&mut h);
                ui.thinking_open.get(&idx).copied().unwrap_or(false).hash(&mut h);
                ui.tool_body_open.get(&idx).copied().unwrap_or(false).hash(&mut h);
                draft_hash(&mut h, ui.wizard_drafts.get(&idx));
                ui.tool_mode.hash(&mut h);
                ui.theme_key.hash(&mut h);
                push(
                    &mut rows,
                    mix(TAG_MESSAGE, msg.ui_id.0),
                    h.finish(),
                    LaneKind::Message(row),
                );
            }
            Entry::Group { start, items, count, tool_name, err_count } => {
                let items: Vec<usize> = items.into_iter().map(|vi| visible[vi]).collect();
                let start_idx = visible[start];
                let open = ui.tool_group_open.get(&start_idx).copied().unwrap_or(false);
                let mut h = hasher();
                TAG_GROUP.hash(&mut h);
                start_idx.hash(&mut h);
                count.hash(&mut h);
                tool_name.hash(&mut h);
                err_count.hash(&mut h);
                for i in &items {
                    content_hash(&msgs[*i]).hash(&mut h);
                    ui.tool_body_open.get(i).copied().unwrap_or(false).hash(&mut h);
                }
                open.hash(&mut h);
                ui.tool_mode.hash(&mut h);
                ui.theme_key.hash(&mut h);
                push(
                    &mut rows,
                    mix(TAG_GROUP, msgs[start_idx].ui_id.0),
                    h.finish(),
                    LaneKind::Group(GroupRow { start_idx, count, tool_name, err_count, items }),
                );
            }
        }
    }

    for q in queued {
        let editing = ui.queue_editing == Some(q.id);
        let mut h = hasher();
        TAG_QUEUED.hash(&mut h);
        q.body.hash(&mut h);
        q.attachments.len().hash(&mut h);
        q.time.hash(&mut h);
        editing.hash(&mut h);
        ui.theme_key.hash(&mut h);
        push(
            &mut rows,
            mix(TAG_QUEUED, q.id),
            h.finish(),
            LaneKind::Queued(QueuedRow { id: q.id, editing }),
        );
    }

    rows
}

impl MessageRow {
    fn hash_into(&self, h: &mut impl Hasher) {
        self.idx.hash(h);
        self.is_last_assistant.hash(h);
        self.is_typing.hash(h);
        self.highlighted.hash(h);
        self.editing.hash(h);
        self.pipeline_tail.hash(h);
        self.wizard_answered.hash(h);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Группировка подряд идущих tool-вызовов одного типа (только minimal-режим)
// ─────────────────────────────────────────────────────────────────────────────

enum Entry {
    Single(usize),
    Group {
        start: usize,
        items: Vec<usize>,
        count: usize,
        tool_name: String,
        err_count: usize,
    },
}

/// Индексы — в пространстве видимых сообщений (`visible[vi]` — исходный).
fn group_lane(msgs: &[ChatMsg], visible: &[usize], tool_mode: &str) -> Vec<Entry> {
    if tool_mode != "minimal" {
        return (0..visible.len()).map(Entry::Single).collect();
    }
    let at = |vi: usize| &msgs[visible[vi]];
    let mut out: Vec<Entry> = Vec::with_capacity(visible.len());
    let mut i = 0usize;
    while i < visible.len() {
        if let Some((name, _)) = pair_at(msgs, visible, i) {
            let name = name.to_string();
            let mut j = i;
            let mut pairs = 0usize;
            let mut errors = 0usize;
            while let Some((n, err)) = pair_at(msgs, visible, j) {
                if n != name {
                    break;
                }
                pairs += 1;
                errors += usize::from(err);
                j += 2;
            }
            if pairs >= 2 {
                out.push(Entry::Group {
                    start: i,
                    items: (i..j).collect(),
                    count: pairs,
                    tool_name: name,
                    err_count: errors,
                });
                i = j;
                continue;
            }
            let _ = at(i);
        }
        out.push(Entry::Single(i));
        i += 1;
    }
    out
}

/// `(инструмент, результат с ошибкой)`, если на `vi` стоит `ToolCall`, а за
/// ним — `ToolResult` того же инструмента.
fn pair_at<'a>(msgs: &'a [ChatMsg], visible: &[usize], vi: usize) -> Option<(&'a str, bool)> {
    if vi + 1 >= visible.len() {
        return None;
    }
    let call_name = match &msgs[visible[vi]].kind {
        // Панель визарда — не карточка инструмента: под шапку группы её не
        // прятать.
        ChatMsgKind::ToolCall { tool_name } if tool_name == "wizard" => return None,
        ChatMsgKind::ToolCall { tool_name } => tool_name.as_str(),
        _ => return None,
    };
    let (result_name, err) = match &msgs[visible[vi + 1]].kind {
        ChatMsgKind::ToolResult { tool_name, error, .. } => (tool_name.as_str(), *error),
        _ => return None,
    };
    if call_name != result_name {
        return None;
    }
    Some((call_name, err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::schema::{ChatToolCall, ChatToolCallFunction};
    use crate::agent::state::MsgAttachment;

    fn ui<'a>(
        tool_mode: &'a str,
        maps: &'a Maps,
    ) -> LaneUi<'a> {
        LaneUi {
            tool_mode,
            pending: false,
            highlight: None,
            editing: None,
            theme_key: 7,
            thinking_open: &maps.thinking,
            tool_body_open: &maps.tool_body,
            tool_group_open: &maps.group,
            compaction_open: &maps.marker,
            wizard_drafts: &maps.drafts,
            queue_editing: None,
        }
    }

    #[derive(Default)]
    struct Maps {
        thinking: HashMap<usize, bool>,
        tool_body: HashMap<usize, bool>,
        group: HashMap<usize, bool>,
        marker: HashMap<u32, bool>,
        drafts: HashMap<usize, WizardDraft>,
    }

    fn assistant(body: &str) -> ChatMsg {
        let mut m = ChatMsg::assistant_empty();
        m.body = body.to_string();
        m
    }
    fn call(name: &str) -> ChatMsg {
        ChatMsg::tool_call(name, "{}", Vec::new())
    }
    fn result(name: &str, error: bool) -> ChatMsg {
        ChatMsg::tool_result("call-id", name, "ok", error)
    }
    fn kinds(rows: &[LaneRow]) -> Vec<&'static str> {
        rows.iter()
            .map(|r| match r.kind {
                LaneKind::DateDivider => "divider",
                LaneKind::Message(_) => "msg",
                LaneKind::Group(_) => "group",
                LaneKind::Marker(_) => "marker",
                LaneKind::Queued(_) => "queued",
            })
            .collect()
    }
    fn queued(id: u64, body: &str) -> QueuedMsg {
        QueuedMsg {
            id,
            chat_id: "c".into(),
            body: body.into(),
            attachments: Vec::new(),
            time: "12:00".into(),
        }
    }

    #[test]
    fn divider_leads_and_queue_trails() {
        let maps = Maps::default();
        let msgs = vec![ChatMsg::user("привет"), assistant("ответ")];
        let rows = build(&msgs, &[queued(1, "в очереди")], &ui("full", &maps));
        assert_eq!(kinds(&rows), vec!["divider", "msg", "msg", "queued"]);
    }

    /// Главное свойство модели: новое сообщение не трогает прежние строки —
    /// иначе мемоизация строк бессмысленна.
    #[test]
    fn appending_a_message_keeps_earlier_rows_intact() {
        let maps = Maps::default();
        let mut msgs = vec![ChatMsg::user("вопрос"), assistant("ответ")];
        let before = build(&msgs, &[], &ui("full", &maps));
        msgs.push(ChatMsg::user("ещё вопрос"));
        let after = build(&msgs, &[], &ui("full", &maps));

        assert_eq!(after.len(), before.len() + 1);
        // Ключи не двигаются вообще, версии — у всех, кроме прежней
        // последней строки: она перестала быть последней репликой
        // ассистента (к ней привязаны стрим-хвост и «Повторить»).
        for (a, b) in before.iter().zip(after.iter()) {
            assert_eq!(a.key, b.key, "ключ строки изменился");
        }
        let last = before.len() - 1;
        for (a, b) in before[..last].iter().zip(after[..last].iter()) {
            assert_eq!(a.version, b.version, "версия строки изменилась");
        }
        assert_ne!(before[last].version, after[last].version);
    }

    /// Но «хвостовые» флаги обязаны обновляться: у карточки pipelines под
    /// хвостом рисуется живой прогон, у визарда — панель ответа.
    #[test]
    fn tail_flags_follow_the_end_of_the_feed() {
        let maps = Maps::default();
        let mut msgs = vec![ChatMsg::user("вопрос"), call("pipelines")];
        let before = build(&msgs, &[], &ui("full", &maps));
        let tail = match &before.last().unwrap().kind {
            LaneKind::Message(m) => m.clone(),
            other => panic!("ожидалось сообщение, получено {other:?}"),
        };
        assert!(tail.pipeline_tail);
        assert!(!tail.wizard_answered);

        msgs.push(result("pipelines", false));
        let after = build(&msgs, &[], &ui("full", &maps));
        let same_row = after.iter().find(|r| r.key == before.last().unwrap().key).unwrap();
        match &same_row.kind {
            LaneKind::Message(m) => assert!(!m.pipeline_tail, "карточка перестала быть хвостом"),
            other => panic!("ожидалось сообщение, получено {other:?}"),
        }
        assert_ne!(same_row.version, before.last().unwrap().version);

        // Визард: пока за результатом вызова ничего нет — ответа не было.
        let mut msgs = vec![ChatMsg::user("вопрос"), call("wizard"), result("wizard", false)];
        let asked = build(&msgs, &[], &ui("full", &maps));
        let wizard_key = asked[2].key;
        match &asked[2].kind {
            LaneKind::Message(m) => assert!(!m.wizard_answered),
            other => panic!("ожидалось сообщение, получено {other:?}"),
        }
        msgs.push(assistant("дальше"));
        let answered = build(&msgs, &[], &ui("full", &maps));
        let row = answered.iter().find(|r| r.key == wizard_key).unwrap();
        match &row.kind {
            LaneKind::Message(m) => assert!(m.wizard_answered, "разговор ушёл дальше"),
            other => panic!("ожидалось сообщение, получено {other:?}"),
        }
    }

    #[test]
    fn compacted_messages_hide_under_their_marker() {
        let maps = Maps::default();
        let mut hidden = ChatMsg::user("старое");
        hidden.compacted_iter = Some(1);
        let msgs = vec![
            ChatMsg::compaction_marker(1, 1, 100, 10, "выжимка"),
            hidden,
            assistant("новое"),
        ];
        let rows = build(&msgs, &[], &ui("full", &maps));
        assert_eq!(kinds(&rows), vec!["divider", "marker", "msg"]);
        match &rows[1].kind {
            LaneKind::Marker(m) => assert_eq!(m.compacted, vec![1]),
            other => panic!("ожидался маркер, получено {other:?}"),
        }
    }

    #[test]
    fn minimal_mode_folds_repeated_tool_pairs() {
        let maps = Maps::default();
        let msgs = vec![
            ChatMsg::user("сделай"),
            call("web"),
            result("web", false),
            call("web"),
            result("web", true),
            assistant("готово"),
        ];
        assert_eq!(
            kinds(&build(&msgs, &[], &ui("minimal", &maps))),
            vec!["divider", "msg", "group", "msg"]
        );
        // В полном режиме каждая карточка — своя строка.
        assert_eq!(
            kinds(&build(&msgs, &[], &ui("full", &maps))),
            vec!["divider", "msg", "msg", "msg", "msg", "msg", "msg"]
        );
        let rows = build(&msgs, &[], &ui("minimal", &maps));
        match &rows[2].kind {
            LaneKind::Group(g) => {
                assert_eq!((g.count, g.err_count, g.tool_name.as_str()), (2, 1, "web"));
                assert_eq!(g.items, vec![1, 2, 3, 4]);
            }
            other => panic!("ожидалась группа, получено {other:?}"),
        }
    }

    /// Раскрытие одной карточки меняет версию только её строки.
    #[test]
    fn opening_one_card_touches_only_its_row() {
        let mut maps = Maps::default();
        let msgs = vec![assistant("первый"), assistant("второй")];
        let before = build(&msgs, &[], &ui("full", &maps));
        maps.thinking.insert(1, true);
        let after = build(&msgs, &[], &ui("full", &maps));
        assert_eq!(after[1].version, before[1].version, "чужая строка задета");
        assert_ne!(after[2].version, before[2].version, "своя строка не обновилась");
    }

    /// Каждое поле сообщения должно доходить до версии строки: иначе
    /// мемоизация покажет устаревший пузырёк.
    #[test]
    fn every_message_field_changes_the_version() {
        let maps = Maps::default();
        let base = ChatMsg::user("тело");
        let version_of = |m: &ChatMsg| build(std::slice::from_ref(m), &[], &ui("full", &maps))[1].version;
        let v0 = version_of(&base);

        let mut cases: Vec<(&str, ChatMsg)> = Vec::new();
        let mut m = base.clone();
        m.body = "другое тело".into();
        cases.push(("body", m));
        let mut m = base.clone();
        m.thinking = "размышление".into();
        cases.push(("thinking", m));
        let mut m = base.clone();
        m.author = "Кто-то".into();
        cases.push(("author", m));
        let mut m = base.clone();
        m.initials = "XX".into();
        cases.push(("initials", m));
        let mut m = base.clone();
        m.tone_class = "avatar-blue".into();
        cases.push(("tone_class", m));
        let mut m = base.clone();
        m.time = "23:59".into();
        cases.push(("time", m));
        let mut m = base.clone();
        m.error = true;
        cases.push(("error", m));
        let mut m = base.clone();
        m.role = ChatMsgRole::Assistant;
        cases.push(("role", m));
        let mut m = base.clone();
        m.kind = ChatMsgKind::ToolCall { tool_name: "web".into() };
        cases.push(("kind", m));
        let mut m = base.clone();
        m.tool_calls = Some(vec![ChatToolCall {
            id: "call-1".into(),
            kind: "function".into(),
            function: ChatToolCallFunction { name: Some("web".into()), arguments: None },
        }]);
        cases.push(("tool_calls", m));
        let mut m = base.clone();
        m.attachments = vec![serde_json::from_value::<MsgAttachment>(serde_json::json!({
            "sha256": "a".repeat(64),
            "mime": "image/png",
        }))
        .expect("вложение из json")];
        cases.push(("attachments", m));
        let mut m = base.clone();
        m.model_note = "заметка".into();
        cases.push(("model_note", m));

        for (field, msg) in cases {
            assert_ne!(version_of(&msg), v0, "поле {field} не дошло до версии строки");
        }
    }

    /// Номер сообщения (`UiId`) в версию не входит: клон той же реплики
    /// показывается так же. Зато ключи строк остаются уникальными.
    #[test]
    fn cloned_message_keeps_version_but_gets_its_own_key() {
        let maps = Maps::default();
        let msg = assistant("один и тот же текст");
        let rows = build(&[msg.clone(), msg.clone()], &[], &ui("full", &maps));
        assert_ne!(rows[1].key, rows[2].key, "ключи строк совпали");
        // Версия повторяема: та же лента даёт те же версии.
        let again = build(&[msg.clone(), msg], &[], &ui("full", &maps));
        assert_eq!(again[1].version, rows[1].version);
        assert_eq!(again[2].version, rows[2].version);
    }

    #[test]
    fn ui_flags_reach_the_version() {
        let maps = Maps::default();
        let msgs = vec![ChatMsg::user("вопрос"), assistant("ответ")];
        let base = build(&msgs, &[], &ui("full", &maps));

        let mut with_highlight = ui("full", &maps);
        with_highlight.highlight = Some(0);
        assert_ne!(build(&msgs, &[], &with_highlight)[1].version, base[1].version);

        let mut editing = ui("full", &maps);
        editing.editing = Some(0);
        assert_ne!(build(&msgs, &[], &editing)[1].version, base[1].version);

        let mut theme = ui("full", &maps);
        theme.theme_key = 42;
        assert_ne!(build(&msgs, &[], &theme)[1].version, base[1].version);

        // «Печатает…»: пустой плейсхолдер ассистента во время генерации.
        let msgs = vec![ChatMsg::user("вопрос"), ChatMsg::assistant_empty()];
        let idle = build(&msgs, &[], &ui("full", &maps));
        let mut pending = ui("full", &maps);
        pending.pending = true;
        let busy = build(&msgs, &[], &pending);
        assert_ne!(busy[2].version, idle[2].version);
        match (&busy[2].kind, &idle[2].kind) {
            (LaneKind::Message(b), LaneKind::Message(i)) => {
                assert!(b.is_typing && !i.is_typing);
                assert!(b.is_last_assistant && i.is_last_assistant);
            }
            _ => panic!("ожидались сообщения"),
        }
    }

    #[test]
    fn queue_row_follows_its_own_state() {
        let maps = Maps::default();
        let q = vec![queued(9, "текст")];
        let base = build(&[], &q, &ui("full", &maps));
        let mut editing = ui("full", &maps);
        editing.queue_editing = Some(9);
        let after = build(&[], &q, &editing);
        assert_eq!(after[1].key, base[1].key);
        assert_ne!(after[1].version, base[1].version);
    }
}
