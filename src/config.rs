use std::env;

pub struct Config {
    pub bind: String,
    pub source_password: String,
    pub admin_password: String,
    pub media_dir: String,
    pub db_path: String,
    pub threads: usize,
    pub public_url: String,
    pub cors_origin: Option<String>,
    pub utc_offset_min: i64,
}

fn var(k: &str, default: &str) -> String {
    env::var(k).unwrap_or_else(|_| default.into())
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            bind: var("RUSTCAST_BIND", "0.0.0.0:8000"),
            source_password: var("RUSTCAST_SOURCE_PASSWORD", "hackme"),
            admin_password: var("RUSTCAST_ADMIN_PASSWORD", "admin"),
            media_dir: var("RUSTCAST_MEDIA_DIR", "storage/media"),
            db_path: var("RUSTCAST_DB", "storage/rustcast.db"),
            threads: var("RUSTCAST_THREADS", "2").parse().unwrap_or(2).max(1),
            public_url: var("RUSTCAST_PUBLIC_URL", "")
                .trim_end_matches('/')
                .to_string(),
            utc_offset_min: var("RUSTCAST_UTC_OFFSET_MIN", "0").parse().unwrap_or(0),
            cors_origin: env::var("RUSTCAST_CORS_ORIGIN")
                .ok()
                .filter(|v| !v.is_empty()),
        }
    }
}
