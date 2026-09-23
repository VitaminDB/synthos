//! End-to-end проверка приёма вложений: реальные файлы → CAS → метаданные,
//! превью и конвертация «под модель».
//!
//! Фикстуры генерирует ffmpeg (`lavfi`-источники), он же используется самим
//! ingest'ом для видео/аудио — если бинаря в системе нет, тест сообщает об
//! этом и выходит, а не падает.
//!
//! Всё делается в одном `#[test]`: ingest адресует CAS через `$HOME`, а
//! переменная окружения одна на процесс — параллельные тесты подрались бы
//! за неё.

use std::path::{Path, PathBuf};
use std::process::Command;

use synthos::agent::state::AttachmentKind;
use synthos::syn_chat::attach::{blobs, ingest};

fn have_ffmpeg() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|bin| {
        Command::new(bin)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// Генерирует фикстуру через lavfi. `None`, если ffmpeg собран без нужного
/// кодека (например, libwebp) — такие проверки просто пропускаем.
fn synth(dir: &Path, name: &str, args: &[&str]) -> Option<PathBuf> {
    let out = dir.join(name);
    let status = Command::new("ffmpeg")
        .args(["-y", "-v", "error"])
        .args(args)
        .arg(&out)
        .status()
        .ok()?;
    (status.success() && out.exists()).then_some(out)
}

#[test]
fn ingests_image_video_audio_and_document() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg/ffprobe не найдены — тест пропущен");
        return;
    }

    let home = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", home.path());
    let fixtures = home.path().join("fixtures");
    std::fs::create_dir_all(&fixtures).unwrap();

    // ── Картинка: PNG читается и моделью, и UI → конвертаций быть не должно.
    let png = synth(
        &fixtures,
        "shot.png",
        &["-f", "lavfi", "-i", "testsrc=size=400x300:rate=1", "-frames:v", "1"],
    )
    .expect("ffmpeg умеет png");
    let a = ingest::ingest(&png).expect("ingest png");
    assert_eq!(a.kind, AttachmentKind::Image);
    assert_eq!((a.width, a.height), (400, 300));
    assert_eq!(a.mime, "image/png");
    assert!(a.model_ext.is_empty(), "PNG не нужно конвертировать для модели");
    assert!(a.ui_ext.is_empty(), "PNG рисуется syngui как есть");
    assert!(
        !a.has_thumb,
        "400×300 меньше потолка thumbnail'а — отдельный файл не нужен"
    );
    assert!(blobs::source_path(&a).exists(), "blob лежит в CAS");
    assert_eq!(blobs::model_path(&a), blobs::source_path(&a));

    // Повторный приём того же файла — тот же хеш, копия не дублируется.
    let again = ingest::ingest(&png).expect("повторный ingest");
    assert_eq!(again.sha256, a.sha256);

    // ── Крупная картинка: должен появиться thumbnail.
    let big = synth(
        &fixtures,
        "big.png",
        &["-f", "lavfi", "-i", "testsrc=size=1600x1200:rate=1", "-frames:v", "1"],
    )
    .expect("ffmpeg умеет png");
    let b = ingest::ingest(&big).expect("ingest big png");
    assert!(b.has_thumb, "1600×1200 больше потолка — нужен thumbnail");
    let thumb = blobs::thumb_path(&b.sha256);
    assert!(thumb.exists(), "thumbnail записан на диск");
    assert_eq!(blobs::preview_path(&b), Some(thumb));

    // ── WebP: модель его читает, а декодер syngui — нет, значит нужна
    //    PNG-копия для UI, но не для модели.
    if let Some(webp) = synth(
        &fixtures,
        "shot.webp",
        &["-f", "lavfi", "-i", "testsrc=size=320x240:rate=1", "-frames:v", "1"],
    ) {
        let w = ingest::ingest(&webp).expect("ingest webp");
        assert_eq!(w.kind, AttachmentKind::Image);
        assert!(w.model_ext.is_empty(), "WebP модель читает сама");
        assert_eq!(w.ui_ext, "png", "для UI нужна PNG-копия");
        assert!(blobs::display_path(&w).exists());
    }

    // ── Видео: размеры и длительность с ffprobe, постер — отдельным кадром.
    let mp4 = synth(
        &fixtures,
        "clip.mp4",
        &[
            "-f", "lavfi", "-i", "testsrc=size=320x240:rate=10", "-t", "2",
            "-pix_fmt", "yuv420p",
        ],
    )
    .expect("ffmpeg умеет mp4");
    let v = ingest::ingest(&mp4).expect("ingest mp4");
    assert_eq!(v.kind, AttachmentKind::Video);
    assert_eq!((v.width, v.height), (320, 240));
    assert!(
        (1_500..=2_500).contains(&v.duration_ms),
        "ожидали ~2000 мс, получили {}",
        v.duration_ms
    );
    assert!(v.has_thumb, "у видео должен быть постер");
    assert!(blobs::thumb_path(&v.sha256).exists());

    // ── Аудио: длительность + WAV 16 кГц моно для ASR.
    let mp3 = synth(
        &fixtures,
        "tone.wav",
        &["-f", "lavfi", "-i", "sine=frequency=440:duration=1"],
    )
    .expect("ffmpeg умеет wav");
    let s = ingest::ingest(&mp3).expect("ingest wav");
    assert_eq!(s.kind, AttachmentKind::Audio);
    assert!((800..=1_200).contains(&s.duration_ms));
    assert_eq!(s.model_ext, "wav");
    assert!(blobs::model_path(&s).exists(), "перекодированный WAV на месте");

    // ── Документ: без конвертаций и превью, зато с типом Document.
    let md = fixtures.join("notes.md");
    std::fs::write(&md, "# Заголовок\n\nтекст").unwrap();
    let d = ingest::ingest(&md).expect("ingest md");
    assert_eq!(d.kind, AttachmentKind::Document);
    assert_eq!(d.mime, "text/markdown");
    assert!(!d.has_thumb);
    assert_eq!(blobs::preview_path(&d), None, "у документа превью-картинки нет");

    // ── GC: пока ни один чат не ссылается на blob'ы, они удаляются.
    blobs::gc_unreferenced(&std::collections::HashSet::new(), std::time::Duration::ZERO);
    assert!(
        !blobs::source_path(&a).exists(),
        "неиспользуемый blob должен быть подчищен"
    );
}
