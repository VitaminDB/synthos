.ffmpeg-player-node {
    background-color: transparent;
}

.ffmpeg-player-body-host {
    background-color: #232631;
    border-radius: 6px;
    padding: 10px;
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.04);
    transition: box-shadow 160ms ease-out;
}

.ffmpeg-player-body-host:hover {
    box-shadow: inset 0 0 0 1px rgba(236, 72, 153, 0.20);
}

/* Кадр ноды — общий плеер (`components::video_player`, стили в
 * video_player.mss) на всю площадь хоста. */
.ffmpeg-player-canvas-host {
    background-color: #000000;
    border-radius: 6px;
    width: 640px;
    height: 480px;
    overflow: hidden;
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.04);
}

.ffmpeg-player-canvas-empty {
    background-color: #000000;
    color: #6B7384;
    border-radius: 4px;
    width: 640px;
    height: 480px;
}

.ffmpeg-player-empty-label {
    color: #6B7384;
    font-size: 13px;
    font-style: italic;
}

.ffmpeg-player-path-host {
    background-color: transparent;
    flex-grow: 1;
    min-width: 120px;
    max-width: 260px;
}
