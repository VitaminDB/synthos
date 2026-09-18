//! Custom title bar for the frameless window.
//!
//! Left: app name + version (`Synthos v<CARGO_PKG_VERSION>`), пилюля стадии
//! («beta») и номер сборки — `pkgrel` из `packaging/PKGBUILD`, который
//! пробрасывает `build.rs`. По нему сразу видно, какая сборка запущена.
//! Right: прогресс загрузок HuggingFace (пока они есть), затем чипы Donate и
//! GitHub вплотную к кнопкам окна (те же ссылки есть в
//! «Настройки → О программе»), затем window controls — either the built-in
//! Windows-style trio or, when «системные кнопки окна» is on, the buttons of
//! the desktop's decoration theme (on KDE with an Aurorae theme they are drawn
//! from its own SVGs, so they match every other window on screen).
//! The whole strip is a WindowDragRegion so the window follows a left-mouse
//! drag over empty areas; the controls sit on top of the drag region and
//! capture their own clicks.

use syngui::appearance::decorations::{read_system_decorations, SystemDecorations, TitleAlignment};
use syngui::open_url;
use syngui::prelude::*;
use syngui::widget::styled::WidgetExt;
use syngui::widgets::overlay::{SystemWindowControls, WindowControl, WindowDragRegion};
use syngui::widgets::visual::{Image, ImageFit};
use syngui::widgets::GestureDetector;
use syngui::window::WindowState;

use crate::context::AppCtx;
use crate::icons::{MI_CLOSE, MI_CLOUD_DOWNLOAD, MI_CROP_SQUARE, MI_FAVORITE, MI_PAUSE, MI_REMOVE};
use crate::pages::huggingface::progress::{self, Totals};
use crate::pages::huggingface::HuggingFaceCtx;

/// Куда ведёт чип Donate — та же ссылка, что в README и `.github/FUNDING.yml`.
pub const DONATE_URL: &str = "https://paypal.me/vitamindbnfkz";
pub const GITHUB_URL: &str = env!("CARGO_PKG_REPOSITORY");

/// Octicons `mark-github` (MIT): белая заливка, цвет даёт `color-tint` из MSS.
/// Холст 64 px при viewBox 16 — растеризуется один раз, и на 12 px в шапке
/// логотип остаётся чётким и на HiDPI.
const GITHUB_MARK_SVG: &[u8] = include_bytes!("../../packaging/github-mark.svg");

/// Зазор между чипами и кнопками окна.
const LINKS_TO_CONTROLS: f32 = 8.0;

pub fn view() -> impl Widget {
    WindowDragRegion::new().child(
        DecoratedBox::new().class("titlebar").child(move || {
            let ctx = use_context::<AppCtx>();
            if ctx.appearance.system_window_controls.get() {
                system_bar(ctx.appearance.window_state.get())
            } else {
                builtin_bar()
            }
        }),
    )
}

/// Номер сборки: пусто, если собирали вне репозитория (см. `build.rs`).
const PKGREL: &str = env!("SYNTHOS_PKGREL");

fn title_text() -> impl Widget {
    let mut row = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            Text::new(tr!("titlebar.title", version = env!("CARGO_PKG_VERSION")))
                .class("titlebar-title"),
        )
        .child(badge(tr!("titlebar.stage.beta"), "titlebar-badge-stage"));
    if !PKGREL.is_empty() {
        row = row.child(badge(format!("#{PKGREL}"), "titlebar-badge-build"));
    }
    DecoratedBox::new().class("titlebar-heading").child(row)
}

/// Всё, что между кнопками окна: заголовок и чипы у правого края.
///
/// При центрированном заголовке распорки по бокам одной ширины, и чипы живут
/// внутри правой: встань они отдельным ребёнком, заголовок сместился бы влево
/// на половину их ширины. `title_inset` — отступ заголовка слева, когда он не
/// по центру.
pub fn middle(centered: bool, title_inset: f32) -> Vec<Box<dyn Widget>> {
    let links_at_end = || {
        Box::new(
            DecoratedBox::new().class("grow").child(
                Row::new()
                    .gap(0.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("grow"))
                    .child(Padding::only(0.0, 0.0, LINKS_TO_CONTROLS, 0.0).child(links())),
            ),
        ) as Box<dyn Widget>
    };
    if centered {
        vec![Box::new(DecoratedBox::new().class("grow")), Box::new(title_text()), links_at_end()]
    } else {
        vec![Box::new(Padding::only(title_inset, 0.0, 0.0, 0.0).child(title_text())), links_at_end()]
    }
}

/// Чипы Donate и GitHub: открывают ссылку в системном браузере. Перед ними —
/// прогресс загрузок с HuggingFace, пока есть что качать.
pub fn links() -> impl Widget {
    Row::new()
        .gap(2.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(downloads_chip())
        .child(link_chip(
            ChipIcon::Glyph(MI_FAVORITE),
            "donate",
            tr!("titlebar.chip.donate"),
            tr!("titlebar.chip.donate.tooltip"),
            || open_link(DONATE_URL),
        ))
        .child(link_chip(
            ChipIcon::GithubMark,
            "github",
            tr!("titlebar.chip.github"),
            tr!("titlebar.chip.github.tooltip"),
            || open_link(GITHUB_URL),
        ))
}

/// Сводный прогресс загрузок в шапке окна: виден с любой страницы, щелчок
/// ведёт на страницу HuggingFace. Пока незавершённых загрузок нет — пусто.
///
/// Свой `Reactive`: `downloads` меняется несколько раз в секунду, и читать его
/// в замыкании самого титлбара значило бы пересобирать всю шапку на каждый тик.
fn downloads_chip() -> impl Widget {
    Reactive::new(|| -> Vec<Box<dyn Widget>> {
        let empty = || vec![Box::new(DecoratedBox::new().class("titlebar-dl-empty")) as Box<dyn Widget>];
        // В тестах шапки контекста страницы нет.
        let Some(hf) = syngui::context_provider::try_use_context::<HuggingFaceCtx>() else {
            return empty();
        };
        let totals = progress::totals(&hf.downloads.get());
        if !totals.has_unfinished() {
            return empty();
        }
        vec![Box::new(downloads_chip_body(&totals, hf.global_paused.get()))]
    })
}

/// Сама пилюля — отдельно от контекста, чтобы собрать её в тесте.
pub fn downloads_chip_body(t: &Totals, paused: bool) -> impl Widget {
    let idle = paused || !t.in_flight();
    let mut label = t.percent_text();
    if !idle && t.speed_bps > 1.0 {
        label.push_str(" · ");
        label.push_str(&progress::human_speed(t.speed_bps));
    }
    let tooltip = crate::pages::huggingface::dock::stats_line(t);
    let state = if idle { " titlebar-dl--idle" } else { "" };
    let fill = DecoratedBox::new()
        .class("titlebar-dl-fill")
        .style("width", syngui::mss::StyleValue::percent(t.ratio().unwrap_or(0.0) * 100.0));
    let chip = DecoratedBox::new().class(format!("titlebar-chip titlebar-dl{state}")).child(
        Center::new().child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(if idle { MI_PAUSE } else { MI_CLOUD_DOWNLOAD }).class("titlebar-dl-icon"))
                .child(DecoratedBox::new().class("titlebar-dl-rail").child(fill))
                .child(Text::new(label).max_lines(1).class("titlebar-dl-text")),
        ),
    );
    Tooltip::new(
        GestureDetector::new()
            .on_click(|| crate::rail::navigate("huggingface"))
            .child(Padding::symmetric(0.0, 6.0).child(chip)),
        tooltip,
    )
}

pub fn open_link(url: &str) {
    if let Err(e) = open_url(url) {
        eprintln!("[titlebar] не удалось открыть {url}: {e}");
    }
}

pub enum ChipIcon {
    Glyph(&'static str),
    GithubMark,
}

/// Чип-ссылка титлбара: иконка и подпись без рамки, по щелчку — `on_click`.
///
/// Наведение ведёт `GestureDetector`, а не `:hover` в MSS: у каждого элемента
/// hover считается по его собственным границам, и `.chip:hover .text` не
/// перекрасил бы подпись, пока курсор над отступами чипа, а у `Text` hover
/// нет вовсе. Подложку всё же зажигает `:hover` самого чипа — его границы
/// совпадают с подложкой, и переход у неё анимируется; цвет иконки и подписи —
/// класс `titlebar-chip--hover`.
///
/// Вертикальный `Padding` внутри детектора растягивает зону щелчка на всю
/// высоту титлбара (32 px при чипе в 20). Нажатие берёт детектор, поэтому
/// окно из-под чипа не начинает перетаскиваться.
pub fn link_chip(
    icon: ChipIcon,
    variant: &'static str,
    label: String,
    tooltip: String,
    on_click: impl FnMut() + Send + 'static,
) -> impl Widget {
    let hovered = use_signal(false);
    let chip = move || {
        let state = if hovered.get() { " titlebar-chip--hover" } else { "" };
        let icon: Box<dyn Widget> = match icon {
            ChipIcon::Glyph(glyph) => Box::new(Icon::new(glyph).class("titlebar-chip-icon")),
            ChipIcon::GithubMark => Box::new(
                Image::from_bytes("titlebar-github-mark", GITHUB_MARK_SVG.to_vec())
                    .fit(ImageFit::Contain)
                    .placeholder(false)
                    .class("titlebar-chip-logo"),
            ),
        };
        DecoratedBox::new()
            .class(format!("titlebar-chip titlebar-chip-{variant}{state}"))
            .child(
                Center::new().child(
                    Row::new()
                        .gap(5.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(vec![
                            icon,
                            Box::new(
                                Text::new(label.clone())
                                    .max_lines(1)
                                    .class("titlebar-badge-text titlebar-chip-text"),
                            ),
                        ]),
                ),
            )
    };
    Tooltip::new(
        GestureDetector::new()
            .on_hover_change(move |inside| hovered.set(inside))
            .on_click(on_click)
            .child(Padding::symmetric(0.0, 6.0).child(DecoratedBox::new().child(chip))),
        tooltip,
    )
}

/// Пилюля титлбара: подложка со скруглением и мелкий текст внутри.
///
/// `Center` обязателен — padding'а мало: он резервирует место, но не
/// выравнивает текст, и в пилюле фиксированной высоты подпись оказывается
/// не по центру. Тот же приём, что у `panel_header::icon_bubble`.
fn badge(text: impl Into<String>, class: &'static str) -> impl Widget {
    DecoratedBox::new().class(format!("titlebar-badge {class}")).child(
        Center::new().child(Text::new(text.into()).max_lines(1).class("titlebar-badge-text")),
    )
}

/// Встроенные кнопки — одинаковы на всех платформах.
fn builtin_bar() -> Row {
    Row::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::Start)
        .children(middle(false, 0.0))
        .child(control_button("min", MI_REMOVE, WindowControl::minimize()))
        .child(control_button("zoom", MI_CROP_SQUARE, WindowControl::toggle_maximize()))
        .child(control_button("close", MI_CLOSE, WindowControl::close()))
}

/// Кнопки из системной темы декораций: раскладка, размеры и внешний вид —
/// как у остальных окон рабочего стола.
fn system_bar(window_state: WindowState) -> Row {
    // Настройки декораций читаются с диска, а титлбар пересобирается на каждый
    // тик реактивного блока — держим один снимок на процесс.
    static DECORATIONS: std::sync::OnceLock<SystemDecorations> = std::sync::OnceLock::new();
    let decorations = DECORATIONS.get_or_init(read_system_decorations).clone();
    let centered = decorations.metrics.title_alignment == TitleAlignment::Center;
    let edge_left = decorations.metrics.edge_left;
    let edge_right = decorations.metrics.edge_right;

    let controls = |side: SystemWindowControls| {
        side.decorations(decorations.clone())
            .maximized(window_state.maximized)
            .active(window_state.focused)
    };

    Row::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .main_axis_alignment(MainAxisAlignment::Start)
        .child(
            Padding::only(edge_left, 0.0, 0.0, 0.0)
                .child(controls(SystemWindowControls::left())),
        )
        .children(middle(centered, 12.0))
        .child(
            Padding::only(0.0, 0.0, edge_right, 0.0)
                .child(controls(SystemWindowControls::right())),
        )
}

fn control_button(variant: &'static str, icon: &'static str, control: WindowControl) -> impl Widget {
    let class = format!("window-control {}", variant);
    control.child(
        DecoratedBox::new().class(class).child(
            Center::new().child(Icon::new(icon).class("window-control-icon")),
        ),
    )
}
