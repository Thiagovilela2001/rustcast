use super::{ApiError, ApiResult, AppState};
use crate::db::models::{Media, Playlist, MEDIA_COLS, PLAYLIST_COLS};
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
    is_jingle: Option<bool>,
    jingle_interval: Option<i64>,
}

pub async fn create(State(s): State<AppState>, Path(id): Path<i64>, Json(p): Json<PlaylistIn>) -> ApiResult<(StatusCode, Json<Playlist>)> {
    let name = p.name.filter(|n| !n.trim().is_empty()).ok_or_else(|| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "name required"))?;
    sqlx::query("SELECT 1 FROM stations WHERE id = ?").bind(id).fetch_one(&s.db).await?;
    let pl = sqlx::query_as(sql!(
        "INSERT INTO playlists (station_id, name, weight, is_jingle, jingle_interval) VALUES (?, ?, ?, ?, ?) RETURNING {PLAYLIST_COLS}"
    ))
    .bind(id)
    .bind(name.trim())
    .bind(p.weight.unwrap_or(10).max(0))
    .bind(p.is_jingle.unwrap_or(false))
    .bind(p.jingle_interval.unwrap_or(0).max(0))
    .fetch_one(&s.db)
    .await?;
    Ok((StatusCode::CREATED, Json(pl)))
}

/// Partial update: omitted fields keep their value.
pub async fn update(State(s): State<AppState>, Path(id): Path<i64>, Json(p): Json<PlaylistIn>) -> ApiResult<Json<Playlist>> {
    Ok(Json(
        sqlx::query_as(sql!(
            "UPDATE playlists SET name = COALESCE(?, name), weight = COALESCE(?, weight), is_jingle = COALESCE(?, is_jingle),
             jingle_interval = COALESCE(?, jingle_interval) WHERE id = ? RETURNING {PLAYLIST_COLS}"
        ))
        .bind(p.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()))
        .bind(p.weight.map(|w| w.max(0)))
        .bind(p.is_jingle)
        .bind(p.jingle_interval.map(|j| j.max(0)))
        .bind(id)
        .fetch_one(&s.db)
        .await?,
    ))
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
