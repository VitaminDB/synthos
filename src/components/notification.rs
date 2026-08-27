//! Notification-pane для synthos: правый НИЖНИЙ угол, над voice-FAB.
//!
//! Использует `syngui::widgets::feedback::NotificationHost` поверх Portal'а
//! с anchor BottomEnd. Default-duration 15 секунд — задаётся в `AppCtx`
//! (см. `crate::context::AppCtx::notifications`).
//!
//! Почему не верхний угол: там живёт шапка страницы с Run/Pause-пилюлей
//! редактора нод — тост её перекрывал ровно в момент запуска графа, когда
//! кнопки нужнее всего.
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

/// Render NotificationHost внутри Portal::BottomEnd, над voice-FAB.
///
/// `margin_bottom = 72` = FAB (44 px) + его нижний отступ (16 px) + 12 px
/// зазор: стек тостов начинается ровно над микрофоном. `margin_right = 16`
/// — по правому краю с FAB'ом.
///
/// `grow_up(true)`: свежий тост появляется снизу, прежние уезжают вверх,
/// колода переполнения выглядывает над верхней карточкой (иначе при нижнем
/// якоре она уходила бы за край окна). Одновременно видно не больше трёх
/// карточек — остальные ждут своей очереди в колоде (MAX_VISIBLE в syngui).
pub fn view(ctx: NotificationCtx) -> impl Widget {
    let always_open = use_signal(true);
    Portal::new()
        .is_open(always_open)
        .modal(false)
        .backdrop(false)
        .anchor(PortalAnchor::BottomEnd {
            margin_bottom: 72.0,
            margin_right: 16.0,
        })
        .child(
            NotificationHost::new(ctx)
                .grow_up(true)
                .class("synthos-notification-host"),
        )
}
