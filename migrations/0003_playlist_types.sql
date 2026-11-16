-- Playlist types/orders (AzuraCast semantics). is_jingle/jingle_interval kept as legacy mirror for old clients.
ALTER TABLE playlists ADD COLUMN playlist_type TEXT NOT NULL DEFAULT 'default';
ALTER TABLE playlists ADD COLUMN play_per INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playlists ADD COLUMN play_per_hour_minute INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playlists ADD COLUMN play_order TEXT NOT NULL DEFAULT 'shuffle';
ALTER TABLE playlists ADD COLUMN played_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE playlists ADD COLUMN queue_pos INTEGER NOT NULL DEFAULT -1;
UPDATE playlists SET playlist_type = 'once_per_x_songs', play_per = jingle_interval WHERE is_jingle = 1 AND jingle_interval > 0;
