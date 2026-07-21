//! Загрузка прикреплённых пользователем файлов через nativ-диалог
//! ([`rfd::AsyncFileDialog`]) и запись в [`super::blobs`] CAS.
//!
//! Полезный side-effect — поддерживать `ChatCtx.draft_attachments` в
//! актуальном состоянии: новые `MsgAttachment` дописываются на main-потоке,
//! без дублей по sha256. Все ошибки логируются, не паникуют.

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::context_provider::use_context;

use crate::chat::{blobs, state::MsgAttachment};
use crate::context::AppCtx;

/// Открыть нативный диалог выбора файлов и прикрепить выбранные картинки
/// к черновику текущего чата.
///
/// Поведение:
/// - Пользователь нажал «Отмена» → молча выходим.
/// - Файл нечитаемый → `log::error!`, остальные обрабатываем.
/// - Дубль по sha256 уже в `draft_attachments` → пропускаем (не добавляем второй раз).
pub fn pick_and_attach() {
    let chat = use_context::<AppCtx>().chat.clone();
    spawn(async move {
        let dialog = rfd::AsyncFileDialog::new()
            .add_filter("Изображения", &["png", "jpg", "jpeg", "webp", "gif", "bmp"])
            .set_title("Выбрать изображения для отправки");
        let Some(handles) = dialog.pick_files().await else {
            return;
        };

        let mut new_atts: Vec<MsgAttachment> = Vec::with_capacity(handles.len());
        for h in handles {
            let path = h.path().to_path_buf();
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(e) => {
                    log::error!("attach: не прочитан {}: {e}", path.display());
                    continue;
                }
            };
            let mime = blobs::sniff_mime(&path, &bytes);
            let original_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            // sha256 + write — CPU-bound, выносим в blocking-pool, чтобы
            // не блокировать reactor на больших файлах.
            let att_result = tokio::task::spawn_blocking(move || {
                blobs::write_if_absent(&bytes, &mime, &original_name)
            })
            .await;
            match att_result {
                Ok(Ok(att)) => new_atts.push(att),
                Ok(Err(e)) => log::error!("attach: blob write failed: {e}"),
                Err(e) => log::error!("attach: blocking task panic: {e}"),
            }
        }

        if new_atts.is_empty() {
            return;
        }

        run_on_main_thread(move || {
            chat.draft_attachments.update(|v| {
                for n in new_atts {
                    if !v.iter().any(|a| a.sha256 == n.sha256) {
                        v.push(n);
                    }
                }
            });
        });
    });
}

/// Удалить прикрепление по индексу из черновика.
pub fn remove_at(idx: usize) {
    let chat = use_context::<AppCtx>().chat.clone();
    chat.draft_attachments.update(|v| {
        if idx < v.len() {
            v.remove(idx);
        }
    });
}
