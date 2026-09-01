//! Реализация точек инъекции DocumentEditor поверх vault'а.
//!
//! Провайдер читает индекс из сигнала `NotesCtx.index` (вызовы приходят
//! с main-потока: пейнт и события редактора), открытие wiki-ссылки
//! открывает/создаёт страницу, url — системный браузер.

use std::sync::Arc;

use syngui::widgets::input::document_editor::{
    DocLinkProvider, LinkCandidate,
};
use syngui::widgets::input::document_editor::LinkTarget;

use super::state::NotesCtx;
use super::storage;

pub struct NotesLinkProvider {
    pub ctx: NotesCtx,
}

pub fn provider(ctx: NotesCtx) -> Arc<NotesLinkProvider> {
    Arc::new(NotesLinkProvider { ctx })
}

impl DocLinkProvider for NotesLinkProvider {
    fn complete(&self, prefix: &str) -> Vec<LinkCandidate> {
        self.ctx
            .index
            .get_untracked()
            .complete(prefix)
            .into_iter()
            .map(|(title, _rel)| LinkCandidate { label: title.clone(), target: title })
            .collect()
    }

    fn link_exists(&self, target: &str) -> bool {
        self.ctx.index.get_untracked().resolve(target).is_some()
    }

    fn open_link(&self, target: &LinkTarget) {
        match target {
            LinkTarget::Wiki { target } => {
                let resolved = self.ctx.index.get_untracked().resolve(target);
                match resolved {
                    Some(rel) => self.ctx.open_path(&rel),
                    None => {
                        // Битая ссылка: создаём страницу с этим именем.
                        let root = self.ctx.vault_path.get_untracked();
                        match storage::create_page(&root, target.trim()) {
                            Ok(rel) => {
                                self.ctx.rescan();
                                self.ctx.reindex_all();
                                self.ctx.open_path(&rel);
                            }
                            Err(e) => log::warn!("notes: не удалось создать {target}: {e}"),
                        }
                    }
                }
            }
            LinkTarget::Url(url) => {
                if let Err(e) = syngui::open_url(url) {
                    log::warn!("notes: не удалось открыть {url}: {e}");
                }
            }
        }
    }
}
