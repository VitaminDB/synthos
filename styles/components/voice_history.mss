/* Страница «История голоса» — список записей слева, плеер справа. */

.voice-history-page {
    background-color: var(--bg-chat);
}

.voice-history-list-pane {
    width: 360px;
    background-color: var(--bg-chats);
    border-right-width: 1px;
    border-right-color: var(--border);
}

.voice-history-detail-pane {
    background-color: var(--bg-chat);
}

.voice-history-list {
    padding: 8px 8px 8px 8px;
}

.voice-history-list-empty {
    padding: 32px 16px 32px 16px;
}

.voice-history-empty-text {
    color: var(--text-muted);
    font-size: 14px;
}

.voice-history-item {
    padding: 12px 14px 12px 14px;
    border-radius: 10px;
    transition: background-color 160ms var(--ease-standard);
}

.voice-history-item:hover {
    background-color: var(--surface-hover);
}

.voice-history-item-selected {
    background-color: var(--surface-selected);
}

.voice-history-item-preview {
    color: var(--text);
    font-size: 14px;
    line-height: 1.35;
    line-clamp: 2;
}

.voice-history-item-meta {
    font-size: 12px;
    color: var(--text-muted);
}

.voice-history-item-meta-sep {
    font-size: 12px;
    color: var(--text-subtle);
}

.voice-history-item-delete {
    color: var(--text-subtle);
}

.voice-history-item-delete:hover {
    color: var(--error);
    background-color: var(--surface-hover);
    border-radius: 8px;
}

.voice-history-detail-empty {
    padding: 32px;
}

.voice-history-detail {
    padding: 24px 28px 24px 28px;
}

.voice-history-player {
    background-color: var(--bg-shell);
    border-radius: 16px;
    padding: 20px 24px 20px 24px;
    border-width: 1px;
    border-color: var(--border-soft);
}

.voice-history-player-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
}

.voice-history-player-model {
    font-size: 13px;
    color: var(--primary);
}

.voice-history-player-meta {
    font-size: 13px;
    color: var(--text-muted);
}

.voice-history-player-meta-sep {
    font-size: 13px;
    color: var(--text-subtle);
}

.voice-history-player-transcript {
    background-color: var(--bg-search);
    border-radius: 10px;
    padding: 14px 16px 14px 16px;
    border-width: 1px;
    border-color: var(--border-soft);
}

.voice-history-player-text {
    color: var(--text);
    font-size: 15px;
    line-height: 1.5;
}

.voice-history-player-progress {
    padding: 0 4px 0 4px;
}

.voice-history-player-controls {
    padding: 4px 0 0 0;
}

.voice-history-player-btn {
    color: var(--text);
}

.voice-history-player-btn:hover {
    background-color: var(--surface-hover);
    border-radius: 8px;
}

.voice-history-player-play  { color: var(--primary); }
.voice-history-player-stop  { color: var(--error); }
