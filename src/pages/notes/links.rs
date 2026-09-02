//! Точки инъекции DocumentEditor: wiki-ссылки поверх дерева проекта.
//!
//! Провайдер читает индекс из сигнала `NotesCtx.index` (вызовы приходят с
//! main-потока: пейнт и события редактора). Открытие wiki-ссылки
//! активирует страницу, битой — создаёт страницу с таким названием в корне;
//! url — системный браузер.

use std::sync::Arc;

use syngui::widgets::input::document_editor::{DocLinkProvider, LinkCandidate, LinkTarget};

use super::index::is_object_target;
use super::state::NotesCtx;

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
            .map(|(title, _id)| LinkCandidate { label: title.clone(), target: title })
            .collect()
    }

    fn link_exists(&self, target: &str) -> bool {
        is_object_target(target) || self.ctx.index.get_untracked().resolve(target).is_some()
    }

    fn open_link(&self, target: &LinkTarget) {
        match target {
            LinkTarget::Wiki { target } => {
                let resolved = self.ctx.index.get_untracked().resolve(target);
                match resolved {
                    Some(id) => self.ctx.activate(&id),
                    None => {
                        // Битая ссылка: создаём страницу с этим названием.
                        self.ctx.create_page(None, target.trim());
                    }
                }
                crate::rail::navigate("notes");
            }
            LinkTarget::Url(url) => {
                if let Err(e) = syngui::open_url(url) {
                    log::warn!("notes: не удалось открыть {url}: {e}");
                }
            }
        }
    }
}
