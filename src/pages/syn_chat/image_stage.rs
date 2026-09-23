//! Сцена картинки в просмотрщике вложений.
//!
//! ```text
//! ┌──────────────────────────────────────────────────┐
//! │ ░░░░ та же картинка: на всю сцену, размыта ░░░░░ │ ← фон вместо чёрных полей
//! │ ░░░ ┌──────────────────────────────┐ ░░░░░░░░░░░ │
//! │  ‹  │          картинка            │  ›          │ ← ImageViewport + листание
//! │ ░░░ └──────────────────────────────┘ ░░░░░░░░░░░ │
//! │          ▢ ▣ ▢ ▢   миниатюры вложений            │
//! │   ( − 100% + │ ⛶ 1:1 ⤢ │ ⟲ ⟳ ⇋ ⇅ │ ⛶ )          │ ← панель инструментов
//! └──────────────────────────────────────────────────┘
//! ```
//!
//! Масштаб, перетаскивание, полосы прокрутки и поворот делает
//! `syngui::ImageViewport`; здесь — слои вокруг него, кнопки и клавиши.
//! Кнопки не трогают вид напрямую: они кладут команду с новым номером в
//! [`ImageSignals::cmd`], карточка просмотрщика пересобирается, и элемент
//! области исполняет команду в `update`.

use syngui::input::{CursorIcon, Key, Modifiers};
use syngui::prelude::*;
use syngui::widgets::containers::GestureDetector;
use syngui::widgets::{ImageViewCommand, ImageViewInfo, ImageViewport};

use crate::components::event_hook::{EventHook, KeyReply};
use crate::icons::{
    MI_CHEVRON_LEFT, MI_CHEVRON_RIGHT, MI_FIT_SCREEN, MI_FLIP, MI_FULLSCREEN, MI_FULLSCREEN_EXIT,
    MI_ROTATE_LEFT, MI_ROTATE_RIGHT, MI_SWAP_VERT, MI_ZOOM_IN, MI_ZOOM_OUT, MI_ZOOM_OUT_MAP,
};
use crate::syn_chat::attach::blobs;
use crate::syn_chat::state::{MsgAttachment, SynChatCtx, ViewerState};

/// Поля области просмотра под то, что лежит поверх неё (см. стили `iv-*`):
/// панель с отступом от низа, лента миниатюр над ней, стрелки по бокам.
const INSET_EDGE: f32 = 16.0;
const INSET_TOOLBAR: f32 = 78.0;
const INSET_STRIP: f32 = 78.0;
const INSET_NAV: f32 = 76.0;

/// Сколько миниатюр видно в ленте разом: текущая держится в середине.
const STRIP_WINDOW: usize = 11;

/// Поворот и отражение. Относятся к картинке, а не к просмотрщику, поэтому
/// помечены её sha: у следующей картинки запись чужая и читается как «без
/// поворота» — сбрасывать ничего не нужно.
#[derive(Clone, PartialEq, Default)]
pub struct Orientation {
    sha: String,
    /// По часовой стрелке, четверти оборота.
    pub turns: i32,
    pub flip_h: bool,
    pub flip_v: bool,
}

/// Состояние сцены, живущее между пересборками карточки.
#[derive(Clone, Copy)]
pub struct ImageSignals {
    /// Команда области просмотра: номер растёт с каждым нажатием.
    pub cmd: RwSignal<(u64, ImageViewCommand)>,
    pub orientation: RwSignal<Orientation>,
    /// Что показывает область: масштаб для подписи, вписано ли.
    pub info: RwSignal<ImageViewInfo>,
    /// Растёт, когда фоновый поток дописал размытый фон очередной картинки.
    pub blur_ready: RwSignal<u64>,
}

impl Default for ImageSignals {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageSignals {
    pub fn new() -> Self {
        Self {
            cmd: use_signal((0, ImageViewCommand::Fit)),
            orientation: use_signal(Orientation::default()),
            info: use_signal(ImageViewInfo::default()),
            blur_ready: use_signal(0),
        }
    }

    pub fn send(&self, command: ImageViewCommand) {
        let seq = self.cmd.get_untracked().0 + 1;
        self.cmd.set((seq, command));
    }

    fn orientation_of(current: Orientation, sha: &str) -> Orientation {
        if current.sha == sha {
            current
        } else {
            Orientation { sha: sha.to_string(), ..Orientation::default() }
        }
    }

    /// Правка ориентации картинки `sha`.
    fn orient(&self, sha: &str, f: impl FnOnce(&mut Orientation)) {
        let mut next = Self::orientation_of(self.orientation.get_untracked(), sha);
        f(&mut next);
        self.orientation.set(next);
    }
}

/// Что сцене нужно от окна просмотрщика.
#[derive(Clone)]
pub struct StageHost {
    pub fullscreen: bool,
    pub toggle_fullscreen: std::sync::Arc<dyn Fn() + Send + Sync>,
    pub toggle_maximize: std::sync::Arc<dyn Fn() + Send + Sync>,
    pub copy: std::sync::Arc<dyn Fn() + Send + Sync>,
}

pub fn image_stage(
    a: &MsgAttachment,
    state: &ViewerState,
    signals: ImageSignals,
    host: StageHost,
) -> impl Widget {
    // Чтение сигналов здесь подписывает на них карточку просмотрщика —
    // она и пересоберёт сцену с новой командой или поворотом.
    let (seq, command) = signals.cmd.get();
    let orientation = ImageSignals::orientation_of(signals.orientation.get(), &a.sha256);

    // Место под панель (и ленту миниатюр над ней) и под стрелки листания:
    // вписанная картинка встаёт над панелью, а не под неё.
    let total = state.items.len();
    let (side, below) = if total > 1 {
        (INSET_NAV, INSET_TOOLBAR + INSET_STRIP)
    } else {
        (INSET_EDGE, INSET_TOOLBAR)
    };

    let path = blobs::display_path(a);
    let viewport = ImageViewport::new(path.display().to_string())
        .natural_size(a.width, a.height)
        .insets(INSET_EDGE, side, below, side)
        .command(seq, command)
        .quarter_turns(orientation.turns)
        .flip(orientation.flip_h, orientation.flip_v)
        .info(signals.info)
        .class("iv-viewport");

    let mut stack = Stack::new()
        .fit(StackFit::Expand)
        .child(backdrop(a, signals))
        .child(DecoratedBox::new().class("iv-scrim"))
        .child(viewport);
    if total > 1 {
        stack = stack.child(nav_layer(-1)).child(nav_layer(1));
    }
    stack = stack.child(bottom_layer(a, state, signals, &host));

    let keys_host = host.clone();
    let fullscreen = host.fullscreen;
    let sha = a.sha256.clone();
    EventHook::new()
        .on_key_down(move |key, mods| on_key(key, mods, signals, &sha, &keys_host, total))
        .capture_keys(move |key| handles_key(key, fullscreen))
        .on_char(move |c| on_char(c, signals))
        .child(DecoratedBox::new().class("iv-stage").child(stack))
}

/// Фон сцены вместо чёрных полей: та же картинка на всю сцену, размытая.
///
/// Размытие делается один раз на процессоре и кладётся рядом с миниатюрами
/// (`<sha>.blur.png`, [`BLUR_SIDE`] px по длинной стороне): при растяжении на
/// сцену такая картинка остаётся гладкой, а рисуется одним прямоугольником.
/// Фильтром GPU (`filter: blur`) так делать нельзя: приложение работает на
/// встроенной графике, и полноэкранные проходы размытия на каждый кадр
/// масштабирования роняли интерфейс до слайд-шоу.
fn backdrop(a: &MsgAttachment, signals: ImageSignals) -> Box<dyn Widget> {
    // Подписка: фоновый поток дёрнет сигнал, когда файл будет готов.
    signals.blur_ready.get();
    match blurred_backdrop(a, signals) {
        Some(path) => Box::new(
            Image::new(path.display().to_string())
                .fit(ImageFit::Cover)
                .placeholder(false)
                .class("iv-backdrop"),
        ),
        None => Box::new(DecoratedBox::new().class("iv-backdrop-empty")),
    }
}

const BLUR_SIDE: u32 = 96;
const BLUR_SIGMA: f32 = 3.0;

pub(super) fn blur_path(sha: &str) -> std::path::PathBuf {
    blobs::thumbs_dir().join(format!("{sha}.blur.png"))
}

/// Готовый размытый фон или `None`, пока он делается в фоне (SVG и то, что
/// не декодируется, остаются без фона — под ними цвет сцены).
fn blurred_backdrop(a: &MsgAttachment, signals: ImageSignals) -> Option<std::path::PathBuf> {
    static IN_FLIGHT: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
        std::sync::Mutex::new(None);

    let dst = blur_path(&a.sha256);
    if dst.exists() {
        return Some(dst);
    }
    if blobs::is_svg(a) {
        return None;
    }
    let src = blobs::preview_path(a)?;
    let sha = a.sha256.clone();
    {
        let mut guard = IN_FLIGHT.lock().ok()?;
        if !guard.get_or_insert_with(Default::default).insert(sha.clone()) {
            // Уже делается (или не получилось) — второй раз не запускаем.
            return None;
        }
    }
    std::thread::spawn(move || match make_blur(&src, &dst) {
        Ok(()) => signals.blur_ready.set(signals.blur_ready.get_untracked() + 1),
        Err(e) => log::warn!("[media-viewer] размытый фон {}: {e}", src.display()),
    });
    None
}

pub(super) fn make_blur(
    src: &std::path::Path,
    dst: &std::path::Path,
) -> std::result::Result<(), String> {
    let img = image::open(src).map_err(|e| e.to_string())?;
    let small = img.thumbnail(BLUR_SIDE, BLUR_SIDE).blur(BLUR_SIGMA).to_rgb8();
    if let Some(dir) = dst.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Через временный файл: наполовину записанный PNG просмотрщик принял бы
    // за готовый фон.
    let tmp = dst.with_extension("tmp");
    small
        .save_with_format(&tmp, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dst).map_err(|e| e.to_string())
}

/// Стрелка листания у края сцены. Слой — строка во всю сцену; пустое место
/// строки клики не ловит, они уходят области просмотра под ней.
fn nav_layer(delta: isize) -> impl Widget {
    let (icon, tip, align) = if delta < 0 {
        (
            MI_CHEVRON_LEFT,
            tr!("chat.media_viewer.prev.tooltip"),
            MainAxisAlignment::Start,
        )
    } else {
        (
            MI_CHEVRON_RIGHT,
            tr!("chat.media_viewer.next.tooltip"),
            MainAxisAlignment::End,
        )
    };
    Row::new()
        .main_axis_alignment(align)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            ToolButton::new(icon)
                .tooltip(tip)
                .on_click(move || step(delta))
                .class("iv-nav"),
        )
}

/// Низ сцены: лента миниатюр (если вложений несколько) и панель инструментов.
fn bottom_layer(
    a: &MsgAttachment,
    state: &ViewerState,
    signals: ImageSignals,
    host: &StageHost,
) -> impl Widget {
    let mut items: Vec<Box<dyn Widget>> = Vec::new();
    if state.items.len() > 1 {
        items.push(Box::new(filmstrip(state)));
    }
    items.push(Box::new(toolbar(a, state, signals, host)));
    items.push(Box::new(DecoratedBox::new().class("iv-bottom-gap")));
    Column::new()
        .gap(10.0)
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(items)
}

/// Окно ленты: до [`STRIP_WINDOW`] миниатюр вокруг текущей.
pub(super) fn strip_range(index: usize, total: usize) -> std::ops::Range<usize> {
    if total <= STRIP_WINDOW {
        return 0..total;
    }
    let half = STRIP_WINDOW / 2;
    let start = index.saturating_sub(half).min(total - STRIP_WINDOW);
    start..start + STRIP_WINDOW
}

fn filmstrip(state: &ViewerState) -> impl Widget {
    let range = strip_range(state.index, state.items.len());
    let thumbs: Vec<Box<dyn Widget>> = range
        .map(|i| {
            let a = &state.items[i];
            let class = if i == state.index {
                "iv-thumb iv-thumb-active"
            } else {
                "iv-thumb"
            };
            let inner: Box<dyn Widget> = match blobs::preview_path(a) {
                Some(p) => Box::new(
                    Image::new(p.display().to_string())
                        .fit(ImageFit::Cover)
                        .placeholder(false)
                        .class("iv-thumb-img"),
                ),
                None => Box::new(
                    Center::new().child(
                        Icon::new(super::attachments::kind_icon(a.kind)).class("iv-thumb-icon"),
                    ),
                ),
            };
            Box::new(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || go_to(i))
                    .child(DecoratedBox::new().class(class).child(inner)),
            ) as Box<dyn Widget>
        })
        .collect();
    DecoratedBox::new().class("iv-strip").child(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(thumbs),
    )
}

fn tool(icon: &'static str, tip: String, on_click: impl Fn() + Send + Sync + 'static) -> ToolButton {
    ToolButton::new(icon).tooltip(tip).on_click(on_click)
}

fn separator() -> impl Widget {
    DecoratedBox::new().class("iv-tool-sep")
}

fn toolbar(
    a: &MsgAttachment,
    state: &ViewerState,
    signals: ImageSignals,
    host: &StageHost,
) -> impl Widget {
    let orient = |f: fn(&mut Orientation)| {
        let sha = a.sha256.clone();
        move || signals.orient(&sha, f)
    };
    let send = move |c: ImageViewCommand| move || signals.send(c);
    let full = host.toggle_fullscreen.clone();
    let (full_icon, full_tip) = if host.fullscreen {
        (
            MI_FULLSCREEN_EXIT,
            tr!("chat.media_viewer.fullscreen_exit.tooltip"),
        )
    } else {
        (MI_FULLSCREEN, tr!("chat.media_viewer.fullscreen.tooltip"))
    };

    let mut items: Vec<Box<dyn Widget>> = Vec::new();
    if state.items.len() > 1 {
        items.push(Box::new(
            Text::new(format!("{} / {}", state.index + 1, state.items.len())).class("iv-counter"),
        ));
        items.push(Box::new(separator()));
    }
    let rest: Vec<Box<dyn Widget>> = vec![
        Box::new(
            tool(
                MI_ZOOM_OUT,
                tr!("chat.media_viewer.zoom_out.tooltip"),
                send(ImageViewCommand::ZoomOut),
            )
            .class("iv-tool"),
        ),
        Box::new(zoom_label(signals)),
        Box::new(
            tool(
                MI_ZOOM_IN,
                tr!("chat.media_viewer.zoom_in.tooltip"),
                send(ImageViewCommand::ZoomIn),
            )
            .class("iv-tool"),
        ),
        Box::new(separator()),
        Box::new(
            tool(
                MI_FIT_SCREEN,
                tr!("chat.media_viewer.fit.tooltip"),
                send(ImageViewCommand::Fit),
            )
            .class("iv-tool"),
        ),
        Box::new(
            Button::new("1:1")
                .on_click(send(ImageViewCommand::Actual))
                .class("iv-tool-text"),
        ),
        Box::new(
            tool(
                MI_ZOOM_OUT_MAP,
                tr!("chat.media_viewer.fill.tooltip"),
                send(ImageViewCommand::Fill),
            )
            .class("iv-tool"),
        ),
        Box::new(separator()),
        Box::new(
            tool(
                MI_ROTATE_LEFT,
                tr!("chat.media_viewer.rotate_left.tooltip"),
                orient(|o| o.turns -= 1),
            )
            .class("iv-tool"),
        ),
        Box::new(
            tool(
                MI_ROTATE_RIGHT,
                tr!("chat.media_viewer.rotate_right.tooltip"),
                orient(|o| o.turns += 1),
            )
            .class("iv-tool"),
        ),
        Box::new(
            tool(
                MI_FLIP,
                tr!("chat.media_viewer.flip_h.tooltip"),
                orient(|o| o.flip_h = !o.flip_h),
            )
            .class("iv-tool"),
        ),
        Box::new(
            tool(
                MI_SWAP_VERT,
                tr!("chat.media_viewer.flip_v.tooltip"),
                orient(|o| o.flip_v = !o.flip_v),
            )
            .class("iv-tool"),
        ),
        Box::new(separator()),
        Box::new(tool(full_icon, full_tip, move || full()).class("iv-tool")),
    ];
    items.extend(rest);

    DecoratedBox::new().class("iv-toolbar").child(
        Row::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(items),
    )
}

/// Подпись масштаба. Отдельный `Reactive`: во время анимации и колеса она
/// меняется каждый кадр, пересобирать ради неё всю карточку незачем.
fn zoom_label(signals: ImageSignals) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let info = signals.info.get();
        vec![Box::new(
            Text::new(format_zoom(info.scale)).class("iv-zoom"),
        )]
    })
}

pub(super) fn format_zoom(scale: f32) -> String {
    let pct = scale * 100.0;
    if pct < 10.0 {
        format!("{pct:.1}%")
    } else {
        format!("{pct:.0}%")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Клавиши
// ─────────────────────────────────────────────────────────────────────────────

/// Клавиши, которые сцена забирает раньше кнопок под фокусом: иначе пробел
/// нажал бы последнюю тронутую кнопку панели, а не листал.
fn handles_key(key: Key, fullscreen: bool) -> bool {
    // Esc во весь экран — выход из него; иначе Esc закрывает просмотрщик
    // (его ловит `Portal`).
    if key == Key::Escape {
        return fullscreen;
    }
    matches!(
        key,
        Key::Left
            | Key::Right
            | Key::Up
            | Key::Down
            | Key::Space
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown
    )
}

fn on_key(
    key: Key,
    mods: Modifiers,
    signals: ImageSignals,
    sha: &str,
    host: &StageHost,
    total: usize,
) -> KeyReply {
    // Приближенную картинку стрелки двигают, вписанную — листают.
    let zoomed = {
        let info = signals.info.get_untracked();
        !info.fit && info.scale > info.fit_scale * 1.01
    };
    const PAN: f32 = 0.15;
    match key {
        Key::C if mods.ctrl => (host.copy)(),
        Key::Left if zoomed => signals.send(ImageViewCommand::PanBy(-PAN, 0.0)),
        Key::Right if zoomed => signals.send(ImageViewCommand::PanBy(PAN, 0.0)),
        Key::Up => signals.send(ImageViewCommand::PanBy(0.0, -PAN)),
        Key::Down => signals.send(ImageViewCommand::PanBy(0.0, PAN)),
        Key::Left | Key::PageUp => step(-1),
        Key::Right | Key::PageDown | Key::Space => step(1),
        Key::Home if total > 1 => go_to(0),
        Key::End if total > 1 => go_to(total - 1),
        Key::Num0 => signals.send(ImageViewCommand::Fit),
        Key::Num1 => signals.send(ImageViewCommand::Actual),
        Key::Num2 => signals.send(ImageViewCommand::Fill),
        Key::R if mods.shift => signals.orient(sha, |o| o.turns -= 1),
        Key::R => signals.orient(sha, |o| o.turns += 1),
        Key::H => signals.orient(sha, |o| o.flip_h = !o.flip_h),
        Key::V => signals.orient(sha, |o| o.flip_v = !o.flip_v),
        Key::F | Key::F11 => (host.toggle_fullscreen)(),
        Key::Escape if host.fullscreen => (host.toggle_fullscreen)(),
        Key::M => (host.toggle_maximize)(),
        _ => return KeyReply::Ignore,
    }
    KeyReply::Handled
}

/// «+» и «−» своих `Key` не имеют — приходят вводом символа, в любой
/// раскладке одинаковым.
fn on_char(c: char, signals: ImageSignals) -> bool {
    match c {
        '+' | '=' => signals.send(ImageViewCommand::ZoomIn),
        '-' | '_' => signals.send(ImageViewCommand::ZoomOut),
        _ => return false,
    }
    true
}

fn step(delta: isize) {
    let ctx = use_context::<SynChatCtx>();
    ctx.viewer.update(|v| {
        if let Some(state) = v.as_mut() {
            state.step(delta);
        }
    });
}

fn go_to(index: usize) {
    let ctx = use_context::<SynChatCtx>();
    ctx.viewer.update(|v| {
        if let Some(state) = v.as_mut() {
            if index < state.items.len() {
                state.index = index;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_keeps_current_in_window() {
        assert_eq!(strip_range(0, 3), 0..3);
        assert_eq!(strip_range(0, 40), 0..11);
        assert_eq!(strip_range(20, 40), 15..26);
        assert_eq!(strip_range(39, 40), 29..40);
        for i in 0..40 {
            assert!(strip_range(i, 40).contains(&i));
        }
    }

    #[test]
    fn blurred_backdrop_is_tiny_and_opaque() {
        let dir = std::env::temp_dir().join(format!("synthos-blur-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.png");
        image::RgbaImage::from_fn(900, 300, |x, _| image::Rgba([(x % 256) as u8, 40, 200, 128]))
            .save(&src)
            .unwrap();
        let dst = dir.join("out.blur.png");
        make_blur(&src, &dst).unwrap();
        let out = image::open(&dst).unwrap();
        assert_eq!((out.width(), out.height()), (96, 32));
        assert!(!out.color().has_alpha(), "фон непрозрачный");
        assert!(!dir.join("out.blur.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zoom_label_keeps_fraction_only_when_tiny() {
        assert_eq!(format_zoom(1.0), "100%");
        assert_eq!(format_zoom(0.756), "76%");
        assert_eq!(format_zoom(0.034), "3.4%");
    }
}
