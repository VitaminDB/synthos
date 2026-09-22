use std::path::{Path, PathBuf};

use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use syngui::tr;
use syngui::widgets::feedback::NotificationCtx;

use crate::config;

use super::state::HuggingFaceCtx;

pub fn is_gguf(filename: &str) -> bool {
    filename.to_ascii_lowercase().ends_with(".gguf")
}

pub fn is_mmproj(filename: &str) -> bool {
    let f = filename.to_ascii_lowercase();
    f.contains("mmproj") && f.ends_with(".gguf")
}

pub fn local_path(ctx: &HuggingFaceCtx, repo_id: &str, filename: &str) -> PathBuf {
    let dir = config::resolve_hf_cache_dir(&ctx.cache_dir.get_untracked());
    dir.join(repo_id).join(filename)
}

pub fn output_path(src: &Path) -> PathBuf {
    src.with_extension("syn")
}

pub fn pick_mmproj(dir: &Path, exclude: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for e in std::fs::read_dir(dir).ok()? {
        let p = e.ok()?.path();
        if p == exclude {
            continue;
        }
        let name = p.file_name()?.to_string_lossy().to_string();
        if is_mmproj(&name) {
            found.push(p);
        }
    }
    let rank = |p: &PathBuf| -> u8 {
        let n = p.to_string_lossy().to_ascii_uppercase();
        if n.contains("F32") {
            0
        } else if n.contains("BF16") {
            1
        } else {
            2
        }
    };
    found.sort_by_key(rank);
    found.into_iter().next()
}

#[cfg(not(feature = "gguf"))]
pub fn start(_ctx: HuggingFaceCtx, notif: NotificationCtx, _repo_id: String, _filename: String) {
    notif.error(tr!("hf.error.gguf_disabled"));
}

#[cfg(feature = "gguf")]
pub fn start(ctx: HuggingFaceCtx, notif: NotificationCtx, repo_id: String, filename: String) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    if ctx.convert_active.get_untracked().is_some() {
        notif.warning(tr!("hf.notify.convert_in_progress"));
        return;
    }
    let src = local_path(&ctx, &repo_id, &filename);
    if !src.is_file() {
        notif.error(tr!("hf.error.file_not_found", path = src.display()));
        return;
    }
    if is_mmproj(&filename) {
        notif.warning(tr!("hf.notify.mmproj_auto"));
        return;
    }
    let out = output_path(&src);
    let mmproj = src.parent().and_then(|d| pick_mmproj(d, &src));

    ctx.convert_active.set(Some(filename.clone()));
    ctx.convert_progress.set(0.0);
    notif.info(tr!("hf.notify.converting", name = filename));

    let active = ctx.convert_active;
    let progress = ctx.convert_progress;
    let notif_done = notif.clone();
    let out_c = out.clone();

    std::thread::spawn(move || {
        let total = Arc::new(AtomicU64::new(1));
        let done = Arc::new(AtomicU64::new(0));
        let last = Arc::new(AtomicU64::new(0));
        let (t, d, l) = (total.clone(), done.clone(), last.clone());
        let cb: synaptix_bundle::ProgressCallback = Arc::new(move |ev| {
            use synaptix_bundle::ProgressEvent as E;
            match ev {
                E::Plan { total_bytes, .. } => t.store(total_bytes.max(1), Ordering::Relaxed),
                E::Bytes { delta } => {
                    let cur = d.fetch_add(delta, Ordering::Relaxed) + delta;
                    let tot = t.load(Ordering::Relaxed);
                    let pct = cur * 1000 / tot.max(1);
                    if pct > l.load(Ordering::Relaxed) {
                        l.store(pct, Ordering::Relaxed);
                        let f = pct as f32 / 1000.0;
                        run_on_main_thread(move || progress.set(f));
                    }
                }
                _ => {}
            }
        });

        // Кванты остаются блоками ggml (`.qpacked` + манифест) — без
        // раздувания Q4 до F16; движок читает их теми же ядрами, что и сам
        // `.gguf`. Нормы и всё, что блоками не отдаётся, — как в Auto.
        let opts = synaptix_gguf::ConvertOptions {
            dtype: synaptix_gguf::OutDtype::Keep,
            mmproj,
            ..Default::default()
        };
        let res = synaptix_gguf::convert_to_syn(&src, &out_c, &opts, Some(cb));

        run_on_main_thread(move || {
            active.set(None);
            progress.set(0.0);
            match res {
                Ok(r) => notif_done.success(tr!(
                    "hf.notify.convert_done",
                    path = r.output.display(),
                    size = format!("{:.1}", r.payload_bytes as f64 / 1e9)
                )),
                Err(e) => notif_done.error(tr!("hf.error.convert_failed", error = e)),
            }
        });
    });
}

pub fn start_from_context(repo_id: String, filename: String) {
    let ctx = use_context::<HuggingFaceCtx>();
    let app = use_context::<crate::context::AppCtx>();
    start(ctx, app.notifications.clone(), repo_id, filename);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_gguf_and_mmproj() {
        assert!(is_gguf("Model-Q8_0.gguf"));
        assert!(is_gguf("Model-Q8_0.GGUF"));
        assert!(!is_gguf("model.safetensors"));
        assert!(is_mmproj("mmproj-F32.gguf"));
        assert!(!is_mmproj("Qwen3.6-27B-MTP-Q8_0.gguf"));
    }

    #[test]
    fn output_replaces_extension() {
        assert_eq!(
            output_path(Path::new("/m/Qwen3.6-27B-MTP-Q8_0.gguf")),
            PathBuf::from("/m/Qwen3.6-27B-MTP-Q8_0.syn")
        );
    }

    #[test]
    fn mmproj_prefers_f32_then_bf16() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["mmproj-F16.gguf", "mmproj-BF16.gguf", "mmproj-F32.gguf", "model.gguf"] {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        let picked = pick_mmproj(dir.path(), &dir.path().join("model.gguf")).unwrap();
        assert!(picked.to_string_lossy().contains("F32"));
    }
}
