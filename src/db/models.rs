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
    /// default | once_per_x_songs | once_per_x_minutes | once_per_hour
    #[serde(rename = "type")]
    pub playlist_type: String,
    /// X for once_per_x_songs / once_per_x_minutes
    pub play_per: i64,
    /// minute of the hour (0-59) for once_per_hour
    pub play_per_hour_minute: i64,
    /// shuffle | random | sequential
    #[serde(rename = "order")]
    pub play_order: String,
    #[serde(skip)]
    pub played_at: i64,
    #[serde(skip)]
    pub queue_pos: i64,
    /// legacy mirror: type != default
    pub is_jingle: bool,
    /// legacy mirror: play_per
    pub jingle_interval: Option<i64>,
}

pub const PLAYLIST_TYPES: [&str; 4] = [
    "default",
    "once_per_x_songs",
    "once_per_x_minutes",
    "once_per_hour",
];
pub const PLAYLIST_ORDERS: [&str; 3] = ["shuffle", "random", "sequential"];

pub const MEDIA_COLS: &str = "id, station_id, file_path, title, artist, album, duration_ms";
pub const STATION_COLS: &str = "id, name, mountpoint, bitrate, crossfade_secs";
pub const PLAYLIST_COLS: &str = "id, station_id, name, weight, playlist_type, play_per, play_per_hour_minute, play_order, played_at, queue_pos, is_jingle, jingle_interval";
