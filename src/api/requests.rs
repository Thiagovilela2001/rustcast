//! Listener song requests. Public endpoints, rate-limited per IP in memory only (IPs never hit the DB),
//! song cooldown + bounded queue. AzuraCast-compatible routes included for existing site widgets.
use super::{ApiError, ApiResult, AppState};
use crate::db::models::{Media, MEDIA_COLS};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;

/// Peer address of the TCP connection, set by the accept loop.
#[derive(Clone, Copy)]
pub struct Peer(pub IpAddr);

const SONG_COOLDOWN_SECS: i64 = 3600;
const MAX_PENDING: i64 = 20;
const MAX_TRACKED_IPS: usize = 10_000;
const KEEP_SECS: i64 = 30 * 24 * 3600;

/// ip -> last request (unix s). Bounded; entries older than the cooldown are pruned.
#[derive(Default)]
pub struct Limiter(Mutex<HashMap<IpAddr, i64>>);

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Behind Caddy the peer is loopback: trust X-Forwarded-For only then.
fn client_ip(peer: Option<Peer>, h: &HeaderMap) -> Option<IpAddr> {
    let peer = peer?.0;
    if peer.is_loopback()
        && let Some(fwd) = h.get("x-forwarded-for").and_then(|v| v.to_str().ok())
    {
        return fwd.split(',').next().and_then(|ip| ip.trim().parse().ok()).or(Some(peer));
    }
    Some(peer)
}

/// Requestable = in the station and not only in special (jingle/ID) playlists.
const REQUESTABLE: &str = "station_id = ? AND id NOT IN (SELECT pm.media_id FROM playlist_media pm JOIN playlists p ON p.id = pm.playlist_id
     WHERE p.playlist_type != 'default' AND pm.media_id NOT IN (SELECT pm2.media_id FROM playlist_media pm2 JOIN playlists p2 ON p2.id = pm2.playlist_id WHERE p2.playlist_type = 'default'))";

#[derive(Deserialize)]
pub struct SearchQ {
    q: Option<String>,
}

/// GET /api/stations/{id}/requests/songs?q=texto — up to 50 requestable tracks.
pub async fn songs(State(s): State<AppState>, Path(id): Path<i64>, Query(q): Query<SearchQ>) -> ApiResult<Json<Vec<Media>>> {
    let like = format!("%{}%", q.q.unwrap_or_default().trim());
    Ok(Json(
        sqlx::query_as(sql!("SELECT {MEDIA_COLS} FROM media WHERE {REQUESTABLE} AND (title LIKE ? OR artist LIKE ?) ORDER BY artist, title LIMIT 50"))
            .bind(id)
            .bind(&like)
            .bind(&like)
            .fetch_all(&s.db)
            .await?,
    ))
}

/// GET /api/stations/{id}/requests — pending queue, oldest first (no IPs).
pub async fn pending(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Vec<Value>>> {
    let rows: Vec<(i64, i64, String, String, i64)> = sqlx::query_as(
        "SELECT r.id, m.id, m.artist, m.title, r.requested_at FROM song_requests r JOIN media m ON m.id = r.media_id
         WHERE r.station_id = ? AND r.played_at IS NULL ORDER BY r.requested_at, r.id",
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(rid, mid, artist, title, at)| json!({ "id": rid, "media_id": mid, "artist": artist, "title": title, "requested_at": at }))
            .collect(),
    ))
}

/// Shared by our route and the AzuraCast-compatible one. Returns the new request id.
async fn do_request(s: &AppState, station_id: i64, media_id: i64, ip: Option<IpAddr>) -> ApiResult<i64> {
    let err = |code, m: String| ApiError::new(code, m);
    if !s.requests_enabled {
        return Err(err(StatusCode::FORBIDDEN, "requests are disabled".into()));
    }
    let ip = ip.ok_or_else(|| err(StatusCode::BAD_REQUEST, "unknown client".into()))?;
    let t = now();
    {
        let map = s.limiter.0.lock().unwrap();
        if let Some(last) = map.get(&ip)
            && t - last < s.request_cooldown_secs
        {
            let wait = s.request_cooldown_secs - (t - last);
            return Err(err(StatusCode::TOO_MANY_REQUESTS, format!("wait {wait} s before requesting again")));
        }
    }
    let ok: Option<(i64,)> = sqlx::query_as(sql!("SELECT id FROM media WHERE id = ? AND {REQUESTABLE}"))
        .bind(media_id)
        .bind(station_id)
        .fetch_optional(&s.db)
        .await?;
    if ok.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "song not requestable on this station".into()));
    }
    let (recent, queued, pending): (i64, i64, i64) = sqlx::query_as(
        "SELECT
           (SELECT COUNT(*) FROM song_history WHERE station_id = ?1 AND media_id = ?2 AND played_at > ?3),
           (SELECT COUNT(*) FROM song_requests WHERE station_id = ?1 AND media_id = ?2 AND played_at IS NULL),
           (SELECT COUNT(*) FROM song_requests WHERE station_id = ?1 AND played_at IS NULL)",
    )
    .bind(station_id)
    .bind(media_id)
    .bind(t - SONG_COOLDOWN_SECS)
    .fetch_one(&s.db)
    .await?;
    if recent > 0 {
        return Err(err(StatusCode::CONFLICT, "song played in the last hour".into()));
    }
    if queued > 0 {
        return Err(err(StatusCode::CONFLICT, "song already requested".into()));
    }
    if pending >= MAX_PENDING {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "request queue is full, try later".into()));
    }
    let (rid,): (i64,) = sqlx::query_as("INSERT INTO song_requests (station_id, media_id, requested_at) VALUES (?, ?, ?) RETURNING id")
        .bind(station_id)
        .bind(media_id)
        .bind(t)
        .fetch_one(&s.db)
        .await?;
    let _ = sqlx::query("DELETE FROM song_requests WHERE played_at < ?").bind(t - KEEP_SECS).execute(&s.db).await;
    let mut map = s.limiter.0.lock().unwrap();
    if map.len() >= MAX_TRACKED_IPS {
        let cd = s.request_cooldown_secs;
        map.retain(|_, last| t - *last < cd);
    }
    map.insert(ip, t);
    Ok(rid)
}

#[derive(Deserialize)]
pub struct RequestIn {
    media_id: i64,
}

/// POST /api/stations/{id}/requests {media_id} — public.
pub async fn create(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    peer: Option<Extension<Peer>>,
    h: HeaderMap,
    Json(r): Json<RequestIn>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let rid = do_request(&s, id, r.media_id, client_ip(peer.map(|e| e.0), &h)).await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": rid }))))
}

/// DELETE /api/requests/{id} — admin cancels a pending request.
pub async fn cancel(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let r = sqlx::query("DELETE FROM song_requests WHERE id = ? AND played_at IS NULL").bind(id).execute(&s.db).await?;
    Ok(if r.rows_affected() == 0 { StatusCode::NOT_FOUND } else { StatusCode::NO_CONTENT })
}

// ---- AzuraCast-compatible ----

async fn station_id(s: &AppState, key: &str) -> ApiResult<i64> {
    let (id,): (i64,) = sqlx::query_as("SELECT id FROM stations WHERE CAST(id AS TEXT) = ? OR mountpoint = ?")
        .bind(key)
        .bind(key)
        .fetch_one(&s.db)
        .await?;
    Ok(id)
}

/// GET /api/station/{station}/requests
pub async fn az_list(State(s): State<AppState>, Path(key): Path<String>) -> ApiResult<Json<Vec<Value>>> {
    let id = station_id(&s, &key).await?;
    let media: Vec<Media> = sqlx::query_as(sql!("SELECT {MEDIA_COLS} FROM media WHERE {REQUESTABLE} ORDER BY artist, title"))
        .bind(id)
        .fetch_all(&s.db)
        .await?;
    let base = &s.public_url;
    Ok(Json(
        media
            .iter()
            .map(|m| {
                json!({
                    "request_id": m.id.to_string(),
                    "request_url": format!("{base}/api/station/{key}/request/{}", m.id),
                    "song": super::azuracast::song(base, m.id, "", Some(m)),
                })
            })
            .collect(),
    ))
}

/// POST /api/station/{station}/request/{request_id} — AzuraCast answers with a Status object.
pub async fn az_request(
    State(s): State<AppState>,
    Path((key, media_id)): Path<(String, i64)>,
    peer: Option<Extension<Peer>>,
    h: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let ip = client_ip(peer.map(|e| e.0), &h);
    let res = async { do_request(&s, station_id(&s, &key).await?, media_id, ip).await }.await;
    match res {
        Ok(_) => (StatusCode::OK, Json(json!({ "success": true, "message": "Your request has been submitted.", "formatted_message": "Your request has been submitted." }))),
        Err(e) => {
            let (code, msg) = e.parts();
            (code, Json(json!({ "success": false, "message": msg, "formatted_message": msg })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarded_for_only_trusted_from_loopback() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "203.0.113.7, 10.0.0.1".parse().unwrap());
        let lo = Some(Peer("127.0.0.1".parse().unwrap()));
        let ext = Some(Peer("198.51.100.2".parse().unwrap()));
        assert_eq!(client_ip(lo, &h), Some("203.0.113.7".parse().unwrap()));
        assert_eq!(client_ip(ext, &h), Some("198.51.100.2".parse().unwrap()), "spoofed header ignored");
        assert_eq!(client_ip(lo, &HeaderMap::new()), Some("127.0.0.1".parse().unwrap()));
    }
}
