/* Глобальный голосовой FAB — две кнопки (corner и center) с MSS-transitions,
 * которые синхронизированы по длительности и easing'у — визуально
 * воспринимаются как одна перемещающаяся и растущая кнопка. */

/* ─── Угловая FAB (правый нижний угол через Portal::BottomEnd) ─── */
.fab-voice-corner {
    width: 44px;
    height: 44px;
    border-radius: 22px;
    icon-size: 22px;
    background-color: var(--primary);
    color: var(--on-primary);
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.28);
    transition: opacity 320ms var(--ease-standard),
                transform 320ms var(--ease-standard),
                background-color 200ms var(--ease-standard);
    animation: voice-fab-idle-pulse 3.0s ease-in-out infinite;
}

.fab-voice-corner:hover {
    background-color: var(--primary-hover);
    transform: scale(1.05);
}

.fab-voice-corner.recording {
    background-color: var(--error);
    animation: none;
}

/* Когда панель открыта — сжимаемся и таем. Центральная FAB-кнопка появляется
 * одновременно с противоположным переходом (scale 0.6→1, opacity 0→1). */
.fab-voice-corner.opening {
    opacity: 0;
    transform: scale(0.0);
    animation: none;
}

@keyframes voice-fab-idle-pulse {
    0%, 100% { box-shadow: 0 4px 14px rgba(0, 0, 0, 0.28); }
    50%      { box-shadow: 0 4px 20px rgba(0, 0, 0, 0.40); }
}

/* ─── Центральная FAB (96×96 внутри voice-overlay-card) ─── */
.fab-voice-center {
    width: 96px;
    height: 96px;
    border-radius: 48px;
    background-color: var(--primary);
    color: var(--on-primary);
    box-shadow: 0 8px 32px rgba(238, 94, 72, 0.55);
    transform: scale(0.6);
    opacity: 0;
    transition: opacity 320ms var(--ease-standard),
                transform 320ms var(--ease-standard),
                background-color 200ms var(--ease-standard);
}

.fab-voice-center.opening {
    transform: scale(1.0);
    opacity: 1.0;
}

.fab-voice-center.recording {
    background-color: var(--error);
}

.fab-voice-center:hover {
    transform: scale(1.05);
}
