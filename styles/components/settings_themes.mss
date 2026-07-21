/* Карточки на странице «Темы». */

.theme-card {
    background-color: var(--bg-panel);
    border-radius: 16px;
    border-width: 1px;
    border-color: var(--border);
    transition: transform var(--duration-med) var(--ease-standard),
                box-shadow var(--duration-med) var(--ease-standard),
                border-color var(--duration-med) var(--ease-standard);
}

.theme-card:hover {
    transform: translateY(-2px);
    box-shadow: 0 18px 32px rgba(24, 24, 43, 0.08);
    border-color: var(--border-strong);
}

.theme-card.active {
    border-color: var(--primary);
    box-shadow: 0 0 0 2px var(--primary-soft);
}

.theme-swatch {
    width: 28px;
    height: 28px;
    border-radius: 999px;
    border-width: 1px;
    border-color: var(--border);
}

.theme-card-name {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
    text-align: center;
}

.theme-apply-btn {
    background-color: var(--primary);
    color: var(--on-primary);
    border-radius: 10px;
    padding: 10px 16px;
    font-size: 14px;
    font-weight: 600;
    transition: background-color var(--duration-fast) var(--ease-standard),
                transform var(--duration-fast) var(--ease-standard);
}

.theme-apply-btn:hover {
    background-color: var(--primary-hover);
    transform: translateY(-1px);
}

.theme-card-badge {
    height: 40px;
    border-radius: 10px;
    background-color: var(--primary-soft);
}

.theme-card-badge-text {
    font-size: 13px;
    font-weight: 700;
    color: var(--primary);
}
