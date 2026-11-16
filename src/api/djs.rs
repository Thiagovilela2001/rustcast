use super::{ApiError, ApiResult, AppState};
use crate::relay::source_handler::hash_password;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

#[derive(sqlx::FromRow, Serialize)]
pub struct Dj {
    id: i64,
    station_id: i64,
    username: String,
    is_active: bool,
}

/// DJ list is admin-only data even though it's a GET.
pub async fn list(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Vec<Dj>>> {
    Ok(Json(
        sqlx::query_as("SELECT id, station_id, username, is_active FROM dj_accounts WHERE station_id = ? ORDER BY username")
            .bind(id)
            .fetch_all(&s.db)
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct DjIn {
    username: Option<String>,
    password: Option<String>,
    is_active: Option<bool>,
}

fn hash(pw: Option<String>) -> ApiResult<Option<String>> {
    match pw {
        None => Ok(None),
        Some(p) if p.len() < 8 => Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "password: min 8 chars")),
        Some(p) => Ok(Some(hash_password(&p))),
    }
}

pub async fn create(State(s): State<AppState>, Path(id): Path<i64>, Json(d): Json<DjIn>) -> ApiResult<(StatusCode, Json<Dj>)> {
    let user = d.username.map(|u| u.trim().to_string()).filter(|u| !u.is_empty() && !u.contains(':'));
    let (Some(user), Some(pw)) = (user, d.password) else {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "username (no ':') and password required"));
    };
    let h = tokio::task::spawn_blocking(move || hash(Some(pw))).await.unwrap()?;
    sqlx::query("SELECT 1 FROM stations WHERE id = ?").bind(id).fetch_one(&s.db).await?;
    let dj = sqlx::query_as(
        "INSERT INTO dj_accounts (station_id, username, password_hash, is_active) VALUES (?, ?, ?, ?)
         RETURNING id, station_id, username, is_active",
    )
    .bind(id)
    .bind(user)
    .bind(h)
    .bind(d.is_active.unwrap_or(true))
    .fetch_one(&s.db)
    .await?;
    Ok((StatusCode::CREATED, Json(dj)))
}

/// Change password and/or enable/disable.
pub async fn update(State(s): State<AppState>, Path(id): Path<i64>, Json(d): Json<DjIn>) -> ApiResult<Json<Dj>> {
    let pw = d.password;
    let h = tokio::task::spawn_blocking(move || hash(pw)).await.unwrap()?;
    Ok(Json(
        sqlx::query_as(
            "UPDATE dj_accounts SET password_hash = COALESCE(?, password_hash), is_active = COALESCE(?, is_active)
             WHERE id = ? RETURNING id, station_id, username, is_active",
        )
        .bind(h)
        .bind(d.is_active)
        .bind(id)
        .fetch_one(&s.db)
        .await?,
    ))
}

pub async fn remove(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let r = sqlx::query("DELETE FROM dj_accounts WHERE id = ?").bind(id).execute(&s.db).await?;
    Ok(if r.rows_affected() == 0 { StatusCode::NOT_FOUND } else { StatusCode::NO_CONTENT })
}
