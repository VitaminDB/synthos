//! Handlers, дёргающие HTTP-апи и обновляющие сигналы контекста.
//! Каждый таск использует `syngui::async_runtime::spawn` (tokio) и затем
//! `run_on_main_thread` для записи результатов в реактивные сигналы.

use syngui::async_runtime::{run_on_main_thread, spawn};
use syngui::context_provider::use_context;

use super::api;
use super::download;
use super::state::{HfModelDetails, HuggingFaceCtx, ListLoadState, TAB_README};

/// Закоммитить текущий `search_query` и перезагрузить список.
pub fn commit_search() {
    let ctx = use_context::<HuggingFaceCtx>();
    let q = ctx.search_query.get_untracked();
    ctx.committed_query.set(q);
    trigger_list_reload();
}

/// Перезагрузить список моделей с актуальными `committed_query` + `sort_mode`.
/// Можно дёргать после смены чипа сорта или из кнопки «Повторить».
pub fn trigger_list_reload() {
    let ctx = use_context::<HuggingFaceCtx>();
    if ctx.list_state.get_untracked() == ListLoadState::Loading {
        return;
    }
    ctx.list_state.set(ListLoadState::Loading);
    ctx.list_error.set(None);

    let query = ctx.committed_query.get_untracked();
    let sort = ctx.sort_mode.get_untracked();
    let gguf_only = ctx.gguf_support.get_untracked() && ctx.gguf_filter.get_untracked();
    let models_sig = ctx.models;
    let state_sig = ctx.list_state;
    let error_sig = ctx.list_error;

    spawn(async move {
        let res = api::list_models(&query, sort, 50, gguf_only).await;
        run_on_main_thread(move || match res {
            Ok(items) => {
                models_sig.set(items);
                error_sig.set(None);
                state_sig.set(ListLoadState::Ready);
            }
            Err(e) => {
                models_sig.set(Vec::new());
                error_sig.set(Some(e.to_string()));
                state_sig.set(ListLoadState::Error);
            }
        });
    });
}

/// Выбрать модель: записать id и асинхронно подгрузить детали + README.
pub fn select_model(repo_id: String) {
    let ctx = use_context::<HuggingFaceCtx>();
    let already = ctx.selected_model.get_untracked();
    if already.as_deref() == Some(repo_id.as_str()) {
        return;
    }
    ctx.selected_model.set(Some(repo_id.clone()));
    ctx.model_details.set(None);
    ctx.readme_text.set(String::new());
    ctx.readme_loading.set(true);
    ctx.detail_tab.set(TAB_README);
    ctx.selected_files.update(|s| s.clear());

    let details_sig = ctx.model_details;
    let readme_sig = ctx.readme_text;
    let readme_loading = ctx.readme_loading;
    let rid = repo_id.clone();

    let rid_for_scan = rid.clone();
    spawn(async move {
        let det = api::get_model_details(&rid).await;
        let rid_scan = rid_for_scan.clone();
        run_on_main_thread(move || match det {
            Ok(d) => {
                // После загрузки details — сканируем кэш на диске, чтобы UI
                // показал уже скачанные файлы как Done (даже после рестарта
                // приложения, когда in-memory map очищена).
                let ctx = use_context::<HuggingFaceCtx>();
                download::scan_existing_files(ctx, &rid_scan, &d.siblings);
                details_sig.set(Some(d));
            }
            // Ошибка деталей — оставляем None; UI отрисует «Не удалось загрузить».
            // Подробный текст не выносим в notification, чтобы не шуметь.
            Err(_) => details_sig.set(Some(HfModelDetails {
                id: rid_clone(&rid_scan),
                siblings: Vec::new(),
                tags: Vec::new(),
                last_modified: None,
                downloads: 0,
                likes: 0,
                author: None,
                pipeline_tag: None,
            })),
        });
    });

    let rid2 = repo_id;
    spawn(async move {
        let readme = api::fetch_readme(&rid2).await.unwrap_or_default();
        run_on_main_thread(move || {
            readme_sig.set(readme);
            readme_loading.set(false);
        });
    });
}

fn rid_clone(s: &str) -> String {
    s.to_string()
}
