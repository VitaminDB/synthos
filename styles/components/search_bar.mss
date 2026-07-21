.search-bar {
    padding: 16px 24px 16px 24px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
    background-color: var(--bg-shell);
}

.search-field {
    background-color: var(--bg-search);
    border-radius: var(--radius-pill);
    padding: 8px 16px 8px 16px;
    height: 40px;
    min-width: 260px;
    max-width: 520px;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.search-field:hover { background-color: #EBEBF0; }

.search-icon { icon-size: 18px; color: var(--text-muted); }
.search-hint { color: var(--text-subtle); font-size: 14px; }

.shortcut-chip {
    padding: 2px 8px 2px 8px;
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border);
    background-color: var(--bg-shell);
    height: 22px;
}

.shortcut-icon { icon-size: 14px; color: var(--text-muted); }
.shortcut-text { color: var(--text-muted); font-size: 12px; font-weight: 500; }

.search-bar-icon { icon-size: 20px; color: var(--text-muted); }

.plan-button {
    padding: 4px 10px 4px 10px;
    border-radius: 8px;
    cursor: pointer;
    transition: background-color var(--duration-fast) var(--ease-standard);
}

.plan-button:hover { background-color: var(--surface-hover); }

.plan-button-icon { icon-size: 18px; color: var(--text-muted); }
.plan-button-text { color: var(--text); font-size: 14px; }
.plan-button-chev { icon-size: 18px; color: var(--text-muted); }
