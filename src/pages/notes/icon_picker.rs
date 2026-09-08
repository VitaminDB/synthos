//! Всплывающая панель выбора иконки страницы: две вкладки — эмодзи по
//! категориям и Material-иконки приложения. Открывается кликом по иконке
//! страницы в дереве или в шапке; выбор пишется в узел дерева
//! (`NotesCtx::set_icon`). Одна панель на режим — якорь и цель в `NotesCtx`.

use syngui::core::Rect;
use syngui::input::CursorIcon;
use syngui::prelude::*;
use syngui::widgets::{GestureDetector, PopupAnchor, PopupPanel};

use crate::icons::*;

use super::state::NotesCtx;

const PANEL_W: f32 = 348.0;
const PANEL_H: f32 = 380.0;
const COLS: usize = 9;

/// Глиф Material-шрифта (Private Use Area) — рисуется `Icon`, иначе это
/// эмодзи и рисуется обычным текстом (цветной шрифт эмодзи).
pub fn is_material_glyph(s: &str) -> bool {
    s.chars()
        .next()
        .map(|c| ('\u{E000}'..='\u{F8FF}').contains(&c))
        .unwrap_or(false)
}

/// Иконка страницы: Material-глиф либо эмодзи.
pub fn render_icon(icon: &str, class: &str) -> impl Widget {
    let inner: Box<dyn Widget> = if is_material_glyph(icon) {
        Box::new(Icon::new(icon.to_string()).class(class))
    } else {
        Box::new(Text::new(icon.to_string()).class(format!("{class} emoji")))
    };
    Stack::new().clip(false).children(vec![inner])
}

/// Открыть панель для страницы, привязав к прямоугольнику иконки.
pub fn open_for(ctx: NotesCtx, page_id: &str, anchor: Rect) {
    ctx.icon_picker_page.set(Some(page_id.to_string()));
    ctx.icon_picker_anchor.set(anchor);
    ctx.icon_picker_open.set(true);
}

pub fn view() -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    let tab = use_signal(0usize);
    PopupPanel::new()
        .is_open(ctx.icon_picker_open)
        .anchor_rect(ctx.icon_picker_anchor)
        .anchor(PopupAnchor::BottomStart)
        .min_width(PANEL_W)
        .max_width(PANEL_W)
        .max_height(PANEL_H)
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            if !ctx.icon_picker_open.get() {
                return vec![Box::new(DecoratedBox::new())];
            }
            vec![Box::new(panel(ctx, tab))]
        }))
        .class("notes-icon-picker")
}

fn panel(ctx: NotesCtx, tab: RwSignal<usize>) -> impl Widget {
    let current = tab.get();
    let header = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("notes-icon-picker-header")
        .child(tab_button(tab, 0, tr!("notes.icon.tab.emoji"), current == 0))
        .child(tab_button(tab, 1, tr!("notes.icon.tab.material"), current == 1))
        .child(DecoratedBox::new().class("grow"))
        .child(
            ToolButton::new(MI_CLOSE)
                .tooltip(tr!("notes.icon.remove"))
                .on_click(move || {
                    if let Some(id) = ctx.icon_picker_page.get_untracked() {
                        ctx.set_icon(&id, None);
                    }
                    ctx.icon_picker_open.set(false);
                }),
        );
    let body: Box<dyn Widget> = if current == 0 {
        Box::new(emoji_grid(ctx))
    } else {
        Box::new(material_grid(ctx))
    };
    let body = Stack::new().clip(false).children(vec![body]);
    Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("notes-icon-picker-body")
        .child(header)
        .child(
            DecoratedBox::new()
                .style("height", syngui::mss::StyleValue::px(PANEL_H - 44.0))
                .child(ScrollView::new().vertical().child(body)),
        )
}

fn tab_button(tab: RwSignal<usize>, idx: usize, label: String, selected: bool) -> impl Widget {
    let class = if selected { "notes-icon-tab selected" } else { "notes-icon-tab" };
    GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click(move || tab.set(idx))
        .child(DecoratedBox::new().class(class).child(Text::new(label).class("notes-icon-tab-label")))
}

fn pick(ctx: NotesCtx, glyph: &str) {
    if let Some(id) = ctx.icon_picker_page.get_untracked() {
        ctx.set_icon(&id, Some(glyph.to_string()));
    }
    ctx.icon_picker_open.set(false);
}

fn grid(ctx: NotesCtx, glyphs: &[&'static str]) -> Column {
    let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Start);
    for chunk in glyphs.chunks(COLS) {
        let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for g in chunk {
            let glyph: &'static str = g;
            row = row.child(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || pick(ctx, glyph))
                    .child(
                        DecoratedBox::new()
                            .class("notes-icon-cell")
                            .child(Center::new().child(render_icon(glyph, "notes-icon-glyph"))),
                    ),
            );
        }
        col = col.child(row);
    }
    col
}

fn emoji_grid(ctx: NotesCtx) -> impl Widget {
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (key, list) in EMOJI_GROUPS {
        col = col.child(Text::new(tr!(key)).class("notes-icon-section"));
        col = col.child(grid(ctx, list));
    }
    DecoratedBox::new().class("notes-icon-grid").child(col)
}

fn material_grid(ctx: NotesCtx) -> impl Widget {
    DecoratedBox::new().class("notes-icon-grid").child(grid(ctx, MATERIAL_ICONS))
}

/// Группы эмодзи (ключ названия в каталоге, список). Общие с панелью
/// эмодзи чата (`pages::syn_chat::emoji_picker`).
pub const EMOJI_GROUPS: &[(&str, &[&str])] = &[
    ("notes.icon.group.smileys", &[
        "😀", "😄", "😁", "😂", "🙂", "😉", "😍", "🤩", "😎", "🤔", "🤨", "😐", "😴", "🤯", "🥳",
        "😇", "🤗", "🤫", "😬", "🙄", "😢", "😡", "🤖", "👻", "💀", "👽", "🎃", "😺", "🙈", "🙉",
    ]),
    ("notes.icon.group.people", &[
        "👋", "👍", "👎", "👏", "🙏", "💪", "✍️", "👀", "🧠", "❤️", "🧑‍💻", "👨‍🔬", "👩‍🎨", "🧑‍🏫", "🧑‍🚀",
        "🧙", "🦸", "👶", "👪", "🗣️", "👤", "👥", "🫶", "🤝", "✌️", "🤞", "👌", "🫡", "💃", "🏃",
    ]),
    ("notes.icon.group.nature", &[
        "🌱", "🌿", "🌳", "🌲", "🌴", "🌵", "🍀", "🌸", "🌺", "🌻", "🌹", "🍁", "🍂", "🌍", "🌙",
        "☀️", "⭐", "🌈", "⚡", "🔥", "💧", "🌊", "❄️", "🐶", "🐱", "🦊", "🐻", "🐼", "🦁", "🐯",
        "🦄", "🐝", "🦋", "🐢", "🐬", "🐙", "🦉", "🐦", "🐘", "🦒",
    ]),
    ("notes.icon.group.food", &[
        "🍎", "🍐", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🍒", "🥑", "🥦", "🌽", "🍞", "🧀", "🍕",
        "🍔", "🌮", "🍣", "🍜", "🍰", "🎂", "🍩", "🍪", "☕", "🍵", "🍺", "🍷", "🥤", "🧊", "🍫",
    ]),
    ("notes.icon.group.activity", &[
        "⚽", "🏀", "🏈", "🎾", "🏐", "🎱", "🏓", "🥊", "🏆", "🥇", "🎯", "🎮", "🎲", "🧩", "🎨",
        "🎬", "🎤", "🎧", "🎸", "🎹", "🥁", "🎻", "🎭", "🎪", "🎟️", "🏋️", "🧘", "🚴", "⛷️", "🏄",
    ]),
    ("notes.icon.group.travel", &[
        "🚗", "🚕", "🚌", "🚲", "🛵", "🚀", "✈️", "🚁", "⛵", "🚂", "🚇", "🗺️", "🧭", "🏠", "🏢",
        "🏫", "🏥", "🏦", "🏭", "🏰", "🗼", "🗽", "⛰️", "🏕️", "🏖️", "🌋", "🏝️", "🛤️", "🌉", "🎡",
    ]),
    ("notes.icon.group.objects", &[
        "💡", "🔦", "🔋", "🔌", "💻", "🖥️", "⌨️", "🖱️", "📱", "📷", "🎥", "📺", "📻", "⏰", "⌛",
        "📚", "📖", "📝", "📌", "📎", "✂️", "📐", "📏", "🔑", "🔒", "🔓", "🔨", "🛠️", "⚙️", "🧲",
        "🧪", "🔬", "🔭", "💊", "💉", "🧬", "📦", "📫", "📁", "📂", "🗂️", "🗒️", "🗓️", "📅", "📊",
        "📈", "📉", "💰", "💳", "🧾", "🛒", "🎁", "🎈", "🏷️", "🔖", "🧵", "🧶", "🪄", "🔮", "🧸",
    ]),
    ("notes.icon.group.symbols", &[
        "✅", "❌", "⚠️", "❓", "❗", "💯", "🔴", "🟠", "🟡", "🟢", "🔵", "🟣", "⚫", "⚪", "🟥",
        "🟧", "🟨", "🟩", "🟦", "🟪", "⬛", "⬜", "🔺", "🔻", "🔶", "🔷", "▶️", "⏸️", "⏹️", "🔁",
        "➕", "➖", "✖️", "➗", "♾️", "💠", "🔰", "⭕", "🚫", "♻️", "🔔", "🔕", "📣", "💬", "💭",
        "🏁", "🚩", "🎌", "🏳️", "🏴", "🔝", "🔙", "🔜", "🆕", "🆗", "🆒", "🆓", "🔞", "㊙️", "🈶",
    ]),
];

const MATERIAL_ICONS: &[&str] = &[
    MI_DESCRIPTION, MI_ARTICLE, MI_BOOK, MI_MENU_BOOK, MI_EDIT_NOTE, MI_LIST_ALT, MI_LIGHTBULB,
    MI_PSYCHOLOGY, MI_AUTO_AWESOME, MI_FAVORITE, MI_BOLT, MI_CODE, MI_TERMINAL, MI_BUG_REPORT,
    MI_DATA_OBJECT, MI_INTEGRATION_INSTRUCTIONS, MI_SETTINGS, MI_TUNE, MI_PALETTE, MI_IMAGE_ICON,
    MI_MOVIE, MI_AUDIOTRACK, MI_MIC, MI_VIDEOCAM, MI_HEADSET_MIC, MI_LIBRARY_MUSIC, MI_GRAPHIC_EQ,
    MI_CHAT, MI_EMAIL, MI_PHONE, MI_CAMPAIGN, MI_NOTIFICATIONS, MI_PERSON, MI_GROUPS, MI_CONTACTS,
    MI_PUBLIC, MI_LANGUAGE, MI_TRAVEL_EXPLORE, MI_FOLDER, MI_FOLDER_OPEN, MI_INBOX, MI_ARCHIVE,
    MI_INVENTORY_2, MI_BAR_CHART, MI_TRENDING_UP, MI_DASHBOARD_CUSTOMIZE, MI_GRID_ON, MI_ACCOUNT_TREE,
    MI_HUB, MI_CHECK, MI_DONE_ALL, MI_TIMER, MI_HISTORY, MI_BOOKMARK_ADD, MI_LOCK, MI_VERIFIED_USER,
    MI_WARNING_AMBER, MI_INFO, MI_HELP_OUTLINE, MI_MEMORY, MI_STORAGE, MI_DNS, MI_ROUTER,
    MI_DESKTOP_WINDOWS, MI_DEVELOPER_BOARD, MI_SPEED, MI_THERMOSTAT, MI_DARK_MODE, MI_LIGHT_MODE,
    MI_SAVE, MI_ATTACH_FILE, MI_PICTURE_AS_PDF, MI_SEARCH, MI_FILTER_ALT, MI_LAUNCH, MI_DOWNLOAD,
    MI_CLOUD_DOWNLOAD, MI_DEPLOYED_CODE, MI_FOLDER_ZIP, MI_APPS, MI_MERGE_TYPE,
];
