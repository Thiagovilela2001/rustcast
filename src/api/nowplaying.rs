//! SSE now playing: one event on connect, then on every title change, plus a refresh every 5 s (listener count).
use super::{ApiResult, AppState};
use crate::db::models::{Station, STATION_COLS};
use crate::relay::mountpoint::MountPoint;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures_util::{stream, Stream};
use std::convert::Infallible;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

async fn next_up(s: &AppState, m: &MountPoint) -> serde_json::Value {
    let id = m.next_up.load(Ordering::SeqCst);
    if id == 0 || m.is_live() {
        return serde_json::Value::Null;
    }
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT artist, title FROM media WHERE id = ?").bind(id).fetch_optional(&s.db).await.ok().flatten();
    row.map_or(serde_json::Value::Null, |(artist, title)| serde_json::json!({ "artist": artist, "title": title, "art_url": format!("/api/media/{id}/art") }))
}

async fn snapshot(s: &AppState, m: &MountPoint) -> serde_json::Value {
    let next = next_up(s, m).await;
    let meta = m.meta.borrow().clone();
    let (artist, title) = match meta.split_once(" - ") {
        Some((a, t)) => (a.to_string(), t.to_string()),
        None => (String::new(), meta),
    };
    let id = m.media_id.load(Ordering::SeqCst);
    let live = m.is_live();
    serde_json::json!({
        "title": title,
        "artist": artist,
        "art_url": (id > 0 && !live).then(|| format!("/api/media/{id}/art")),
        "listeners": m.listeners(),
        "source": if live { "live" } else { "autodj" },
        "next": next,
    })
}

async fn mount_of(s: &AppState, id: i64) -> ApiResult<Arc<MountPoint>> {
    let st: Station = sqlx::query_as(sql!("SELECT {STATION_COLS} FROM stations WHERE id = ?")).bind(id).fetch_one(&s.db).await?;
    Ok(s.mounts.get_or_create(&st.mountpoint))
}

pub async fn sse(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let mount = mount_of(&s, id).await?;
    let rx = mount.meta.subscribe();
    let events = stream::unfold((s, mount, rx, true), |(s, m, mut rx, first)| async move {
        if !first && let Ok(Err(_)) = tokio::time::timeout(Duration::from_secs(5), rx.changed()).await {
            return None; // sender gone
        }
        rx.borrow_and_update();
        let ev = Event::default().json_data(snapshot(&s, &m).await).unwrap();
        Some((Ok(ev), (s, m, rx, false)))
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

/// GET /api/stations/{id}/history?limit=20 — newest first (AutoDJ + live), from the DB.
pub async fn history(State(s): State<AppState>, Path(id): Path<i64>, Query(q): Query<HistQ>) -> ApiResult<Json<Vec<serde_json::Value>>> {
    let rows = crate::db::history::recent(&s.db, id, q.limit.unwrap_or(20)).await?;
    Ok(Json(rows.into_iter().map(|r| {
        let (artist, title) = r.text.split_once(" - ").map_or((String::new(), r.text.clone()), |(a, t)| (a.into(), t.into()));
        serde_json::json!({
            "id": r.id, "media_id": r.media_id, "artist": artist, "title": title, "played_at": r.played_at,
            "is_live": r.is_live, "listeners": r.listeners, "art_url": r.media_id.map(|m| format!("/api/media/{m}/art")),
        })
    }).collect()))
}

#[derive(serde::Deserialize)]
pub struct HistQ {
    limit: Option<i64>,
}

/// Plain JSON variant (same payload) for clients without SSE.
pub async fn json(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<serde_json::Value>> {
    let m = mount_of(&s, id).await?;
    Ok(Json(snapshot(&s, &m).await))
}
