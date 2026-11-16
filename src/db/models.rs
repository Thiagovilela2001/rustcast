use serde::Serialize;

#[derive(sqlx::FromRow, Serialize, Clone)]
pub struct Station {
    pub id: i64,
    pub name: String,
    pub mountpoint: String,
    pub bitrate: i64,
    pub crossfade_secs: f64,
}

#[derive(sqlx::FromRow, Serialize, Clone)]
pub struct Media {
    pub id: i64,
    pub station_id: i64,
    #[serde(skip)]
    pub file_path: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: i64,
}

#[derive(sqlx::FromRow, Serialize, Clone)]
pub struct Playlist {
    pub id: i64,
    pub station_id: i64,
    pub name: String,
    pub weight: i64,
    pub is_jingle: bool,
    pub jingle_interval: Option<i64>,
}

pub const MEDIA_COLS: &str = "id, station_id, file_path, title, artist, album, duration_ms";
pub const STATION_COLS: &str = "id, name, mountpoint, bitrate, crossfade_secs";
pub const PLAYLIST_COLS: &str = "id, station_id, name, weight, is_jingle, jingle_interval";
