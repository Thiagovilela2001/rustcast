-- Listener requests. No IP stored (rate limit lives in memory).
CREATE TABLE song_requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    station_id INTEGER NOT NULL REFERENCES stations(id) ON DELETE CASCADE,
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    requested_at INTEGER NOT NULL,
    played_at INTEGER
);
CREATE INDEX idx_song_requests_pending ON song_requests (station_id, played_at, requested_at);
