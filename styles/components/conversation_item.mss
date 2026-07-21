.conversation-item {
    padding: 10px 16px 10px 16px;
    border-radius: 12px;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.conversation-item:hover    { background-color: var(--surface-hover); }
.conversation-item.selected { background-color: var(--surface-selected); }

.conv-name {
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
}

.conv-preview {
    color: var(--text-muted);
    font-size: 13px;
    /* Длинные tool-call'ы / JSON в превью раздували карточку на 5–7 строк.
     * `line-clamp` (syngui ≥ 2026-04) обрезает на 3-й строке с эллипсисом. */
    line-clamp: 3;
}

/* Avatar palette. */
Avatar.avatar-orange { background-color: var(--avatar-orange); color: #6F3B21; }
Avatar.avatar-green  { background-color: var(--avatar-green);  color: #2F5F3B; }
Avatar.avatar-blue   { background-color: var(--avatar-blue);   color: #274064; }
Avatar.avatar-violet { background-color: var(--avatar-violet); color: #3E2A63; }
Avatar.avatar-rose   { background-color: var(--avatar-rose);   color: #6A2A31; }
Avatar.avatar-slate  { background-color: var(--avatar-slate);  color: #2A2F3A; }

/* Presence dot — sits over bottom-right of the avatar via Stack + Padding. */
.presence-dot {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border-width: 2px;
    border-color: var(--bg-chats);
}

.presence-dot.presence-online     { background-color: var(--presence-online); }
.presence-dot.presence-offline    { background-color: var(--presence-offline); }
.presence-dot.presence-instagram  { background-color: var(--presence-instagram); }
.presence-dot.presence-messenger  { background-color: var(--presence-messenger); }
.presence-dot.presence-whatsapp   { background-color: var(--presence-whatsapp); }
.presence-dot.presence-telegram   { background-color: var(--presence-telegram); }

.unread-badge {
    background-color: var(--primary);
    border-radius: var(--radius-pill);
    padding: 2px 8px 2px 8px;
    min-width: 20px;
    height: 20px;
}

.unread-badge-text {
    color: var(--on-primary);
    font-size: 12px;
    font-weight: 600;
}
