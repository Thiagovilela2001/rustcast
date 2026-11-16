use super::{ApiError, ApiResult, AppState};
use crate::db::models::{Media, Playlist, MEDIA_COLS, PLAYLIST_COLS, PLAYLIST_ORDERS, PLAYLIST_TYPES};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

pub async fn list(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Vec<Playlist>>> {
    Ok(Json(sqlx::query_as(sql!("SELECT {PLAYLIST_COLS} FROM playlists WHERE station_id = ? ORDER BY id")).bind(id).fetch_all(&s.db).await?))
}

#[derive(Deserialize)]
pub struct PlaylistIn {
    name: Option<String>,
    weight: Option<i64>,
    #[serde(rename = "type")]
    playlist_type: Option<String>,
    play_per: Option<i64>,
    play_per_hour_minute: Option<i64>,
    order: Option<String>,
    // legacy (still accepted): is_jingle=true == type once_per_x_songs, jingle_interval == play_per
    is_jingle: Option<bool>,
    jingle_interval: Option<i64>,
}

struct Resolved {
    playlist_type: Option<String>,
    play_per: Option<i64>,
    minute: Option<i64>,
    order: Option<String>,
}

fn resolve(p: &PlaylistIn) -> ApiResult<Resolved> {
    let bad = |m: &str| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, m.to_string());
    let playlist_type = match (&p.playlist_type, p.is_jingle) {
        (Some(t), _) => Some(t.clone()),
        (None, Some(true)) => Some("once_per_x_songs".into()),
        (None, Some(false)) => Some("default".into()),
        (None, None) => None,
    };
    if let Some(t) = &playlist_type
        && !PLAYLIST_TYPES.contains(&t.as_str())
    {
        return Err(bad("type: default | once_per_x_songs | once_per_x_minutes | once_per_hour"));
    }
    if let Some(o) = &p.order
        && !PLAYLIST_ORDERS.contains(&o.as_str())
    {
        return Err(bad("order: shuffle | random | sequential"));
    }
    if p.play_per_hour_minute.is_some_and(|m| !(0..=59).contains(&m)) {
        return Err(bad("play_per_hour_minute: 0-59"));
    }
    Ok(Resolved {
        playlist_type,
        play_per: p.play_per.or(p.jingle_interval).map(|v| v.max(0)),
        minute: p.play_per_hour_minute,
        order: p.order.clone(),
    })
}

/// Keep legacy columns mirroring the new ones, then return the row.
async fn sync_legacy(s: &AppState, id: i64) -> ApiResult<Playlist> {
    sqlx::query("UPDATE playlists SET is_jingle = (playlist_type != 'default'), jingle_interval = play_per WHERE id = ?")
        .bind(id)
        .execute(&s.db)
        .await?;
    Ok(sqlx::query_as(sql!("SELECT {PLAYLIST_COLS} FROM playlists WHERE id = ?")).bind(id).fetch_one(&s.db).await?)
}

pub async fn create(State(s): State<AppState>, Path(id): Path<i64>, Json(p): Json<PlaylistIn>) -> ApiResult<(StatusCode, Json<Playlist>)> {
    let r = resolve(&p)?;
    let name = p.name.filter(|n| !n.trim().is_empty()).ok_or_else(|| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "name required"))?;
    sqlx::query("SELECT 1 FROM stations WHERE id = ?").bind(id).fetch_one(&s.db).await?;
    let (new_id,): (i64,) = sqlx::query_as(
        "INSERT INTO playlists (station_id, name, weight, playlist_type, play_per, play_per_hour_minute, play_order)
         VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(id)
    .bind(name.trim())
    .bind(p.weight.unwrap_or(10).max(0))
    .bind(r.playlist_type.unwrap_or_else(|| "default".into()))
    .bind(r.play_per.unwrap_or(0))
    .bind(r.minute.unwrap_or(0))
    .bind(r.order.unwrap_or_else(|| "shuffle".into()))
    .fetch_one(&s.db)
    .await?;
    Ok((StatusCode::CREATED, Json(sync_legacy(&s, new_id).await?)))
}

/// Partial update: omitted fields keep their value.
pub async fn update(State(s): State<AppState>, Path(id): Path<i64>, Json(p): Json<PlaylistIn>) -> ApiResult<Json<Playlist>> {
    let r = resolve(&p)?;
    let res = sqlx::query(
        "UPDATE playlists SET name = COALESCE(?, name), weight = COALESCE(?, weight), playlist_type = COALESCE(?, playlist_type),
         play_per = COALESCE(?, play_per), play_per_hour_minute = COALESCE(?, play_per_hour_minute), play_order = COALESCE(?, play_order)
         WHERE id = ?",
    )
    .bind(p.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()))
    .bind(p.weight.map(|w| w.max(0)))
    .bind(r.playlist_type)
    .bind(r.play_per)
    .bind(r.minute)
    .bind(r.order)
    .bind(id)
    .execute(&s.db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound.into());
    }
    Ok(Json(sync_legacy(&s, id).await?))
}

pub async fn remove(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let r = sqlx::query("DELETE FROM playlists WHERE id = ?").bind(id).execute(&s.db).await?;
    Ok(if r.rows_affected() == 0 { StatusCode::NOT_FOUND } else { StatusCode::NO_CONTENT })
}

pub async fn list_media(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Vec<Media>>> {
    let cols = MEDIA_COLS.split(", ").map(|c| format!("m.{c}")).collect::<Vec<_>>().join(", ");
    Ok(Json(
        sqlx::query_as(sql!(
            "SELECT {cols} FROM media m JOIN playlist_media pm ON pm.media_id = m.id WHERE pm.playlist_id = ? ORDER BY pm.position, m.id"
        ))
        .bind(id)
        .fetch_all(&s.db)
        .await?,
    ))
}

#[derive(Deserialize)]
pub struct AddMedia {
    media_id: i64,
}

pub async fn add_media(State(s): State<AppState>, Path(id): Path<i64>, Json(a): Json<AddMedia>) -> ApiResult<StatusCode> {
    // Media and playlist must belong to the same station.
    let r = sqlx::query(
        "INSERT OR IGNORE INTO playlist_media (playlist_id, media_id, position)
         SELECT p.id, m.id, COALESCE((SELECT MAX(position) + 1 FROM playlist_media WHERE playlist_id = p.id), 0)
         FROM playlists p JOIN media m ON m.station_id = p.station_id WHERE p.id = ? AND m.id = ?",
    )
    .bind(id)
    .bind(a.media_id)
    .execute(&s.db)
    .await?;
    if r.rows_affected() == 0 {
        let exists = sqlx::query("SELECT 1 FROM playlist_media WHERE playlist_id = ? AND media_id = ?").bind(id).bind(a.media_id).fetch_optional(&s.db).await?;
        if exists.is_none() {
            return Err(ApiError::new(StatusCode::NOT_FOUND, "playlist or media not found in same station"));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_media(State(s): State<AppState>, Path((id, media_id)): Path<(i64, i64)>) -> ApiResult<StatusCode> {
    sqlx::query("DELETE FROM playlist_media WHERE playlist_id = ? AND media_id = ?").bind(id).bind(media_id).execute(&s.db).await?;
    Ok(StatusCode::NO_CONTENT)
}
