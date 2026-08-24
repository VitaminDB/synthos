.node-card.vibevoice-node {
    width: 400px;
}

.vibevoice-node-running {
    animation: vibevoice-pulse 1100ms ease-in-out infinite alternate;
}

@keyframes vibevoice-pulse {
    0%   { opacity: 0.60; }
    100% { opacity: 1.00; }
}
