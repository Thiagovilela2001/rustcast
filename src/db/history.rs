//! Song history: one DB row per title change on air (AutoDJ tracks and live sets). Nothing kept in RAM.
use crate::relay::mountpoint::MountPoint;
use serde::Serialize;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::sync::atomic::Ordering;

const KEEP_SECS: i64 = 30 * 24 * 3600;

#[derive(sqlx::FromRow, Serialize, Clone)]
pub struct HistoryRow {
    pub id: i64,
    pub media_id: Option<i64>,
    pub text: String,
    pub played_at: i64,
    pub is_live: bool,
    pub listeners: i64,
}

pub async fn recent(db: &SqlitePool, station_id: i64, limit: i64) -> sqlx::Result<Vec<HistoryRow>> {
    sqlx::query_as(
        "SELECT id, media_id, text, played_at, is_live, listeners FROM song_history WHERE station_id = ? ORDER BY played_at DESC, id DESC LIMIT ?",
    )
    .bind(station_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(db)
    .await
}

/// Watches the mount's title and writes a row on every change. Call from inside the tokio runtime.
pub fn start_recorder(db: SqlitePool, mount: Arc<MountPoint>, station_id: i64) {
    let mut rx = mount.meta.subscribe();
    tokio::spawn(async move {
        let mut inserts = 0u32;
        while rx.changed().await.is_ok() {
            let text = rx.borrow_and_update().clone();
            if text.is_empty() {
                continue;
            }
            let media_id = Some(mount.media_id.load(Ordering::SeqCst)).filter(|&id| id > 0);
            let played_at = mount.started_at.load(Ordering::SeqCst) as i64;
            let r = sqlx::query("INSERT INTO song_history (station_id, media_id, text, played_at, is_live, listeners) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(station_id)
                .bind(media_id)
                .bind(&text)
                .bind(played_at)
                .bind(mount.is_live())
                .bind(mount.listeners() as i64)
                .execute(&db)
                .await;
            if let Err(e) = r {
                eprintln!("history: {e}");
            }
            inserts += 1;
            if inserts % 100 == 1 {
                let _ =
                    sqlx::query("DELETE FROM song_history WHERE station_id = ? AND played_at < ?")
                        .bind(station_id)
                        .bind(played_at - KEEP_SECS)
                        .execute(&db)
                        .await;
            }
        }
    });
}
