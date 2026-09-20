//! Просмотрщик вложений в отдельном окне — посмотреть глазами без запуска
//! всего приложения. HOME подменяется временным каталогом: blob'ы и конфиг
//! настоящего пользователя не трогаются.
//!
//! `cargo run --example viewer_demo -- картинка1.png [картинка2.jpg …]`

use syngui::prelude::*;
use synthos::pages::settings::theme_data::default_dark_theme;
use synthos::syn_chat::attach::blobs;
use synthos::syn_chat::state::{AttachmentKind, MsgAttachment, ViewerState};
use synthos::syn_chat::SynChatCtx;

fn main() {
    let paths: Vec<std::path::PathBuf> = std::env::args().skip(1).map(Into::into).collect();
    assert!(!paths.is_empty(), "укажите хотя бы одну картинку");

    let home = std::env::temp_dir().join(format!("synthos-viewer-demo-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);

    let items: Vec<MsgAttachment> = paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (width, height) = image::image_dimensions(p).unwrap_or((0, 0));
            let mut a = MsgAttachment {
                sha256: format!("viewer-demo-{i}"),
                mime: "image/png".into(),
                original_name: p.file_name().unwrap().to_string_lossy().into_owned(),
                width,
                height,
                size_bytes: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
                kind: AttachmentKind::Image,
                ext: p.extension().unwrap_or_default().to_string_lossy().to_lowercase(),
                duration_ms: 0,
                model_ext: String::new(),
                ui_ext: String::new(),
                has_thumb: false,
                share_path: false,
            };
            let blob = blobs::source_path(&a);
            std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
            std::fs::copy(p, &blob).expect("копия картинки в blob-каталог");
            // Миниатюра — как при настоящем прикреплении: из неё просмотрщик
            // делает размытый фон и ленту.
            if width.max(height) > blobs::THUMB_MAX_SIDE {
                if let (Ok(img), Ok(dst)) = (image::open(p), blobs::prepare_thumb(&a.sha256)) {
                    let side = blobs::THUMB_MAX_SIDE;
                    a.has_thumb = img.thumbnail(side, side).save(dst).is_ok();
                }
            }
            a
        })
        .collect();

    let theme = use_signal(default_dark_theme().to_mss());
    App::new()
        .title("Synthos viewer demo")
        .size(1700, 1050)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(synthos::styles::styles())
        .with_dynamic_theme(theme)
        .run(move |_| {
            provide_context(SynChatCtx::new());
            use_context::<SynChatCtx>().viewer.set(Some(ViewerState {
                items: items.clone(),
                index: 0,
            }));
            Box::new(
                Stack::new()
                    .fit(StackFit::Expand)
                    .child(DecoratedBox::new().class("page-root"))
                    .child(synthos::pages::syn_chat::media_viewer::view()),
            )
        });
}
