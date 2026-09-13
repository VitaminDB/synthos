/* Плавающее окно чата, плейсхолдер на странице и кнопка-аватар свёрнутого
 * окна (`pages::syn_chat::float_window`). */

/* Окно. padding — рамка вокруг ленты: с нулём квадратные углы ленты
 * вылезали бы за скруглённые углы окна. min-* — нижняя граница ресайза
 * мышью; стартовый размер — `CHAT_WINDOW_DEFAULT_SIZE`.
 * Тень двухслойная: ближняя даёт контур на светлом фоне, дальняя
 * (`--glass-shadow`, темы задают её сами) — отрыв от страницы. */
.chat-float-window {
    background-color: var(--bg-panel);
    color: var(--text);
    border: 1px solid var(--border-strong);
    border-radius: 14px;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.18), var(--glass-shadow);
    padding: 8px;
    min-width: 360px;
    min-height: 320px;
    font-size: 13px;
}

/* Полоса над лентой в окне: «печатает…» слева, переход к панелям чата
 * справа (`float_window::window_bar`). */
.chat-float-window-bar {
    height: 30px;
    padding: 0px 0px 4px 6px;
}

.chat-float-window-typing {
    font-size: 12px;
    color: var(--text-subtle);
}

/* Бейджик «модель печатает» (`chat_item::typing_dot`): точка поверх
 * аватара в рейле и на кнопке свёрнутого окна, пульсирует, пока идёт ход.
 * Обводка цветом подложки отделяет её от аватара. */
.chat-typing-dot {
    width: 10px;
    height: 10px;
    border-radius: 5px;
    background-color: var(--primary);
    border: 2px solid var(--bg-rail);
    animation: chat-typing-pulse 1.2s ease-in-out infinite;
}

@keyframes chat-typing-pulse {
    0%, 100% { opacity: 1.0; }
    50%      { opacity: 0.35; }
}

/* Тело закрытого окна — пустышка без размера. */
.chat-float-window-empty {
    width: 0px;
    height: 0px;
}

/* ─── Плейсхолдер в центральной колонке страницы ─── */
.chat-detached-placeholder {
    background-color: var(--bg-chat);
}

.chat-detached-card {
    padding: 32px;
    max-width: 440px;
}

.chat-detached-icon {
    icon-size: 40px;
    color: var(--text-subtle);
}

.chat-detached-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
}

.chat-detached-hint {
    font-size: 12px;
    color: var(--text-subtle);
    text-align: center;
}

/* ─── Кнопка-аватар свёрнутого окна (слева внизу) ─── */
.fab-chat-corner {
    width: 52px;
    height: 52px;
    border-radius: 26px;
    background-color: var(--bg-panel);
    border: 1px solid var(--border-soft);
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.28);
    transition: transform 200ms var(--ease-standard),
                box-shadow 200ms var(--ease-standard);
}

.fab-chat-corner:hover {
    transform: scale(1.06);
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.36);
}

/* Идёт генерация — ответ приходит в свёрнутое окно, кнопка пульсирует. */
.fab-chat-corner.generating {
    animation: chat-fab-pulse 1.6s ease-in-out infinite;
}

@keyframes chat-fab-pulse {
    0%, 100% { box-shadow: 0 4px 14px rgba(0, 0, 0, 0.28); }
    50%      { box-shadow: 0 0 0 6px var(--primary-soft); }
}
