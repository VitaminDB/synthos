.chat-pane-wrap {
    background-color: var(--bg-chat);
}


.message-area {
    background-color: var(--bg-chat);
}

.message-area-scroll-wrap {
    padding: 20px 24px 20px 24px;
}

.dot-pattern {
    background-color: var(--bg-chat);
    color: var(--bg-chat-dots);
    width: 100%;
    height: 100%;
}

/* Hero-блок пустого чата. Появляется только когда история сообщений пуста. */
.msg-hero {
    padding: 64px 32px 64px 32px;
    background-color: transparent;
    animation: hero-fade-in var(--duration-med) var(--ease-standard);
}

.msg-hero-title {
    color: var(--text);
    font-size: 18px;
    font-weight: 600;
}

.msg-hero-sub {
    color: var(--text-muted);
    font-size: 14px;
}

@keyframes hero-fade-in {
    0%   { opacity: 0; transform: translateY(6px); }
    100% { opacity: 1; transform: translateY(0); }
}

/* Пузырёк, на который привёл глобальный поиск: рамка держится, пока
 * пользователь не переключит чат — по ней видно, какое именно сообщение
 * нашлось, когда лента длинная. */
.msg-search-highlight {
    border-width: 1px;
    border-color: var(--primary);
    border-radius: 14px;
    background-color: var(--primary-soft);
    padding: 4px;
}
