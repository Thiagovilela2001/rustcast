CREATE TABLE song_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    station_id INTEGER NOT NULL REFERENCES stations(id) ON DELETE CASCADE,
    media_id INTEGER REFERENCES media(id) ON DELETE SET NULL,
    text TEXT NOT NULL,
    played_at INTEGER NOT NULL,
    is_live BOOLEAN NOT NULL DEFAULT 0,
    listeners INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_song_history_station_time ON song_history (station_id, played_at DESC);
