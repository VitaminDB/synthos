/* Notification host — стек уведомлений в правом верхнем углу.
 * Размещение и margin от title-bar контролируются Portal'ом
 * (см. components/notification.rs::view), здесь только стиль карточек.
 *
 * Severity-классы автоматически добавляются виджетом:
 *   .severity-info / .severity-success / .severity-warning / .severity-error
 * Каждый задаёт `accent-color` — левый бордер 3px у карточки.
 */

Notification {
    /* Контент-driven ширина: max 25% viewport (syngui резолвит percent
     * относительно containing-block, передаваемого Portal'ом). */
    max-width: 25%;
    gap: 12px;
    padding: 14px 16px;
    border-radius: 12px;
    background-color: var(--bg-shell);
    color: var(--text);
    font-size: 14px;
    /* Transitions для opacity/transform применяются на каждом frame'е через
     * fade animation в element. Эти MSS значения — будущая интеграция
     * с keyframe slide-in справа. */
    transition: opacity var(--duration-med, 200ms) var(--ease-standard),
                transform var(--duration-med, 200ms) var(--ease-standard);
}

/* Severity accent colors — переопределяют fallback из syngui.
 * Виджет читает `accent-color` для левого бордера каждой карточки. */
Notification.severity-info    { accent-color: var(--primary, #3B82F6); }
Notification.severity-success { accent-color: #22C55E; }
Notification.severity-warning { accent-color: #F59E0B; }
Notification.severity-error   { accent-color: var(--error, #EF4444); }

/* Synthos-специфичный класс для дополнительной кастомизации (если потребуется). */
.synthos-notification-host {
    /* Резерв под frameless title-bar и гap справа управляется Portal'ом
     * (margin_top/margin_right в notification.rs). */
}
