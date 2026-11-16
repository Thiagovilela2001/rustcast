//! AzuraCast-compatible now playing, so existing site players work by only swapping the domain.
//! Station key = id or shortcode (our shortcode is the mountpoint).
use super::{ApiResult, AppState};
use crate::db::models::{MEDIA_COLS, Media, STATION_COLS, Station};
use axum::Json;
use axum::extract::{Path, State};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

async fn station(s: &AppState, key: &str) -> ApiResult<Station> {
    Ok(sqlx::query_as(sql!(
        "SELECT {STATION_COLS} FROM stations WHERE CAST(id AS TEXT) = ? OR mountpoint = ?"
    ))
    .bind(key)
    .bind(key)
    .fetch_one(&s.db)
    .await?)
}

pub(crate) fn song(base: &str, id: i64, text: &str, media: Option<&Media>) -> Value {
    let (artist, title) = match media {
        Some(md) => (md.artist.clone(), md.title.clone()),
        None => text
            .split_once(" - ")
            .map_or((String::new(), text.to_string()), |(a, t)| {
                (a.into(), t.into())
            }),
    };
    json!({
        "id": id.to_string(), "text": if artist.is_empty() { title.clone() } else { format!("{artist} - {title}") },
        "artist": artist, "title": title, "album": media.and_then(|md| md.album.clone()).unwrap_or_default(),
        "genre": "", "isrc": "", "lyrics": "", "custom_fields": [],
        "art": format!("{base}/api/media/{id}/art?fallback=1")
    })
}

async fn media_by_id(s: &AppState, id: i64) -> Option<Media> {
    if id <= 0 {
        return None;
    }
    sqlx::query_as(sql!("SELECT {MEDIA_COLS} FROM media WHERE id = ?"))
        .bind(id)
        .fetch_optional(&s.db)
        .await
        .ok()
        .flatten()
}

async fn np(s: &AppState, st: &Station) -> Value {
    let m = s.mounts.get_or_create(&st.mountpoint);
    let base = &s.public_url;
    let text = m.meta.borrow().clone();
    let id = m.media_id.load(Ordering::SeqCst);
    let live = m.is_live();
    let media: Option<Media> = if id > 0 && !live {
        sqlx::query_as(sql!("SELECT {MEDIA_COLS} FROM media WHERE id = ?"))
            .bind(id)
            .fetch_optional(&s.db)
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    let (artist, title) = match &media {
        Some(md) => (md.artist.clone(), md.title.clone()),
        None => match text.split_once(" - ") {
            Some((a, t)) => (a.to_string(), t.to_string()),
            None => (String::new(), text.clone()),
        },
    };
    let started = m.started_at.load(Ordering::SeqCst) as i64;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let duration = media.as_ref().map_or(0, |md| md.duration_ms / 1000);
    let elapsed = if started > 0 {
        (now - started).max(0)
    } else {
        0
    };
    let elapsed = if duration > 0 {
        elapsed.min(duration)
    } else {
        elapsed
    };
    let next_id = m.next_up.load(Ordering::SeqCst);
    let playing_next = match (live, media_by_id(s, next_id).await) {
        (false, Some(md)) => json!({
            "cued_at": now, "played_at": started + duration, "duration": md.duration_ms / 1000, "playlist": "", "is_request": false,
            "song": song(base, md.id, "", Some(&md)),
        }),
        _ => Value::Null,
    };
    // last 5 before the current one
    let rows = crate::db::history::recent(&s.db, st.id, 6)
        .await
        .unwrap_or_default();
    let mut song_history = Vec::new();
    for r in rows.iter().skip(1) {
        let md = match r.media_id {
            Some(mid) => media_by_id(s, mid).await,
            None => None,
        };
        song_history.push(json!({
            "sh_id": r.id, "played_at": r.played_at, "duration": md.as_ref().map_or(0, |x| x.duration_ms / 1000),
            "playlist": "", "streamer": if r.is_live { r.text.as_str() } else { "" }, "is_request": false,
            "song": song(base, r.media_id.unwrap_or(0), &r.text, md.as_ref()),
        }));
    }
    let n = m.listeners();
    let listeners = json!({ "total": n, "unique": n, "current": n });
    let path = format!("/listen/{}/radio.mp3", st.mountpoint);
    let listen_url = format!("{base}{path}");
    json!({
        "station": {
            "id": st.id, "name": st.name, "shortcode": st.mountpoint, "description": "", "frontend": "rustcast", "backend": "rustcast",
            "listen_url": listen_url, "url": base, "public_player_url": format!("{base}/"), "is_public": true,
            "mounts": [{ "id": 1, "name": "MP3", "url": listen_url, "path": path, "bitrate": st.bitrate, "format": "mp3", "is_default": true, "listeners": listeners }],
            "remotes": [], "hls_enabled": false, "hls_url": null, "playlist_pls_url": null, "playlist_m3u_url": null
        },
        "listeners": listeners,
        "live": { "is_live": live, "streamer_name": if live { text.as_str() } else { "" }, "broadcast_start": if live { Some(started) } else { None }, "art": null },
        "now_playing": {
            "sh_id": 0, "played_at": started, "duration": duration, "playlist": "", "streamer": if live { text.as_str() } else { "" }, "is_request": false,
            "elapsed": elapsed, "remaining": (duration - elapsed).max(0),
            "song": {
                "id": id.to_string(), "text": if artist.is_empty() { title.clone() } else { format!("{artist} - {title}") },
                "artist": artist, "title": title, "album": media.as_ref().and_then(|md| md.album.clone()).unwrap_or_default(),
                "genre": "", "isrc": "", "lyrics": "", "custom_fields": [],
                "art": format!("{base}/api/media/{id}/art?fallback=1")
            }
        },
        "playing_next": playing_next, "song_history": song_history, "is_online": true, "cache": null
    })
}

/// GET /api/nowplaying
pub async fn all(State(s): State<AppState>) -> ApiResult<Json<Vec<Value>>> {
    let stations: Vec<Station> =
        sqlx::query_as(sql!("SELECT {STATION_COLS} FROM stations ORDER BY id"))
            .fetch_all(&s.db)
            .await?;
    let mut out = Vec::with_capacity(stations.len());
    for st in &stations {
        out.push(np(&s, st).await);
    }
    Ok(Json(out))
}

/// GET /api/nowplaying/{id|shortcode} and /api/nowplaying_static/{shortcode}.json
pub async fn one(State(s): State<AppState>, Path(key): Path<String>) -> ApiResult<Json<Value>> {
    let st = station(&s, key.trim_end_matches(".json")).await?;
    Ok(Json(np(&s, &st).await))
}
