.about-identity-card {
    padding: 24px;
}

.about-identity-logo {
    width: 96px;
    height: 96px;
    border-radius: 24px;
    background-color: var(--primary-soft);
    padding: 8px;
}

.about-identity-name {
    color: var(--text);
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.5px;
}

.about-identity-version {
    color: var(--primary);
    font-size: 14px;
    font-weight: 600;
    letter-spacing: 0.5px;
}

.about-identity-tagline {
    color: var(--text-muted);
    font-size: 14px;
    line-height: 1.4;
}

.about-link-button {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background-color: var(--surface-hover);
    transition: background-color var(--duration-fast) var(--ease-standard),
                color var(--duration-fast) var(--ease-standard);
}

.about-link-button:hover {
    background-color: var(--primary-soft);
}

.about-link-icon {
    color: var(--text-muted);
    icon-size: 18px;
    transition: color var(--duration-fast) var(--ease-standard);
}

.about-link-button:hover .about-link-icon {
    color: var(--primary);
}
