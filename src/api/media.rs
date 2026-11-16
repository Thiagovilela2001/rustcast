use super::{ApiError, ApiResult, AppState};
use crate::db::models::{Media, MEDIA_COLS};
use axum::extract::{Multipart, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use lofty::prelude::*;
use tokio::io::AsyncWriteExt;

pub async fn list(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Vec<Media>>> {
    Ok(Json(sqlx::query_as(sql!("SELECT {MEDIA_COLS} FROM media WHERE station_id = ? ORDER BY artist, title")).bind(id).fetch_all(&s.db).await?))
}

/// Keep only safe chars: no path traversal, no surprises on Windows.
fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || ".-_".contains(c) { c } else { '_' }).collect();
    let s = s.trim_start_matches('.');
    if s.is_empty() { "upload".into() } else { s.chars().take(100).collect() }
}

struct Tags {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    duration_ms: i64,
}

fn read_tags(path: &std::path::Path) -> Result<Tags, lofty::error::FileParseError> {
    let f = lofty::read_from_path(path)?;
    let tag = f.primary_tag().or_else(|| f.first_tag());
    let get = |v: Option<std::borrow::Cow<str>>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    Ok(Tags {
        title: tag.and_then(|t| get(t.title())),
        artist: tag.and_then(|t| get(t.artist())),
        album: tag.and_then(|t| get(t.album())),
        duration_ms: f.properties().duration().as_millis() as i64,
    })
}

pub async fn upload(State(s): State<AppState>, Path(id): Path<i64>, mut mp: Multipart) -> ApiResult<(StatusCode, Json<Media>)> {
    sqlx::query("SELECT 1 FROM stations WHERE id = ?").bind(id).fetch_one(&s.db).await?;
    let bad = |e: axum::extract::multipart::MultipartError| ApiError::new(StatusCode::BAD_REQUEST, e.to_string());
    while let Some(mut field) = mp.next_field().await.map_err(bad)? {
        if field.name() != Some("file") {
            continue;
        }
        let fname = sanitize(field.file_name().unwrap_or("upload"));
        let dir = s.media_dir.join(id.to_string());
        tokio::fs::create_dir_all(&dir).await.map_err(io)?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = dir.join(format!("{nanos}_{fname}"));
        let mut file = tokio::fs::File::create(&path).await.map_err(io)?;
        let written = async {
            while let Some(chunk) = field.chunk().await.map_err(bad)? {
                file.write_all(&chunk).await.map_err(io)?;
            }
            file.flush().await.map_err(io)
        }
        .await;
        let p = path.clone();
        let tags = match written {
            Ok(()) => tokio::task::spawn_blocking(move || read_tags(&p)).await.ok().and_then(|r| r.ok()),
            Err(e) => {
                let _ = tokio::fs::remove_file(&path).await;
                return Err(e);
            }
        };
        let Some(t) = tags else {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "not a readable audio file"));
        };
        let stem = std::path::Path::new(&fname).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let m: Media = sqlx::query_as(sql!(
            "INSERT INTO media (station_id, file_path, title, artist, album, duration_ms) VALUES (?, ?, ?, ?, ?, ?) RETURNING {MEDIA_COLS}"
        ))
        .bind(id)
        .bind(path.to_string_lossy())
        .bind(t.title.unwrap_or(stem))
        .bind(t.artist.unwrap_or_else(|| "Unknown".into()))
        .bind(t.album)
        .bind(t.duration_ms)
        .fetch_one(&s.db)
        .await?;
        return Ok((StatusCode::CREATED, Json(m)));
    }
    Err(ApiError::new(StatusCode::BAD_REQUEST, "multipart field `file` missing"))
}

fn io(e: std::io::Error) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub async fn remove(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let m: Media = sqlx::query_as(sql!("DELETE FROM media WHERE id = ? RETURNING {MEDIA_COLS}")).bind(id).fetch_one(&s.db).await?;
    let _ = tokio::fs::remove_file(&m.file_path).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Cover art read from tags on demand (not stored separately).
pub async fn art(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Response> {
    let m: Media = sqlx::query_as(sql!("SELECT {MEDIA_COLS} FROM media WHERE id = ?")).bind(id).fetch_one(&s.db).await?;
    let pic = tokio::task::spawn_blocking(move || {
        let f = lofty::read_from_path(&m.file_path).ok()?;
        let p = f.tags().iter().flat_map(|t| t.pictures()).next()?.clone();
        Some((p.mime_type().map(|m| m.as_str().to_string()).unwrap_or_else(|| "image/jpeg".into()), p.into_data()))
    })
    .await
    .ok()
    .flatten();
    match pic {
        Some((mime, data)) => Ok(([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "max-age=86400".into())], data).into_response()),
        None => Err(ApiError::new(StatusCode::NOT_FOUND, "no cover art")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sanitize_blocks_traversal() {
        assert_eq!(super::sanitize("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(super::sanitize("..\\win.ini"), "_win.ini");
        assert_eq!(super::sanitize("Minha Música.mp3"), "Minha_M_sica.mp3");
        assert_eq!(super::sanitize("..."), "upload");
    }
}
