//! SSE now playing: one event on connect, then on every title change, plus a refresh every 5 s (listener count).
use super::{ApiResult, AppState};
use crate::db::models::{Station, STATION_COLS};
use crate::relay::mountpoint::MountPoint;
use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures_util::{stream, Stream};
use std::convert::Infallible;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

fn snapshot(m: &MountPoint) -> serde_json::Value {
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
    })
}

async fn mount_of(s: &AppState, id: i64) -> ApiResult<Arc<MountPoint>> {
    let st: Station = sqlx::query_as(sql!("SELECT {STATION_COLS} FROM stations WHERE id = ?")).bind(id).fetch_one(&s.db).await?;
    Ok(s.mounts.get_or_create(&st.mountpoint))
}

pub async fn sse(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let mount = mount_of(&s, id).await?;
    let rx = mount.meta.subscribe();
    let events = stream::unfold((mount, rx, true), |(m, mut rx, first)| async move {
        if !first && let Ok(Err(_)) = tokio::time::timeout(Duration::from_secs(5), rx.changed()).await {
            return None; // sender gone
        }
        rx.borrow_and_update();
        let ev = Event::default().json_data(snapshot(&m)).unwrap();
        Some((Ok(ev), (m, rx, false)))
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

/// Plain JSON variant (same payload) for clients without SSE.
pub async fn json(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<serde_json::Value>> {
    let m = mount_of(&s, id).await?;
    Ok(Json(snapshot(&m)))
}
