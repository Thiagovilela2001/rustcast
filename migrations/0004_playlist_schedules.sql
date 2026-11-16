-- Time windows for playlists (station local time). A playlist with rows here plays only inside them.
CREATE TABLE playlist_schedules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    start_min INTEGER NOT NULL,          -- minutes after local midnight, 0-1439
    end_min INTEGER NOT NULL,            -- end < start = crosses midnight
    days TEXT NOT NULL DEFAULT '1234567',-- ISO weekdays of the START day, Mon=1
    start_date TEXT,                     -- optional YYYY-MM-DD (inclusive)
    end_date TEXT
);
CREATE INDEX idx_playlist_schedules_playlist ON playlist_schedules (playlist_id);
