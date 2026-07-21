//! Notification-pane для synthos: правый верхний угол под frameless title-bar.
//!
//! Использует `syngui::widgets::feedback::NotificationHost` поверх Portal'а
//! с anchor TopEnd. Default-duration 15 секунд — задаётся в `AppCtx`
//! (см. `crate::context::AppCtx::notifications`).
//!
//! Application-уровень API:
//! ```ignore
//! let app = use_context::<AppCtx>();
//! app.notifications.info("Готово");
//! app.notifications.error("Не удалось загрузить");
//! ```

use syngui::prelude::*;
use syngui::widgets::feedback::{NotificationCtx, NotificationHost};
use syngui::widgets::overlay::PortalAnchor;

/// Render NotificationHost внутри Portal::TopEnd с отступом под title-bar.
/// `margin_top = 40` = высота title-bar (32 px) + 8 px зазор. `margin_right = 12`
/// — небольшой gap от правого края окна.
pub fn view(ctx: NotificationCtx) -> impl Widget {
    let always_open = use_signal(true);
    Portal::new()
        .is_open(always_open)
        .modal(false)
        .backdrop(false)
        .anchor(PortalAnchor::TopEnd {
            margin_top: 40.0,
            margin_right: 12.0,
        })
        .child(NotificationHost::new(ctx).class("synthos-notification-host"))
}
