pub mod azuracast;
pub mod djs;
pub mod media;
pub mod nowplaying;
pub mod playlists;
pub mod requests;

use crate::db::models::{STATION_COLS, Station};
use crate::db::scheduler::start_autodj;
use crate::relay::{mountpoint::Mounts, source_handler::check_basic};
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use sqlx::SqlitePool;
use std::path::PathBuf;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub mounts: Mounts,
    pub media_dir: PathBuf,
    pub admin_password: String,
    /// e.g. https://radio.blackmesa.works — used for absolute URLs in AzuraCast-compatible JSON.
    pub public_url: String,
    pub requests_enabled: bool,
    pub request_cooldown_secs: i64,
    pub limiter: std::sync::Arc<requests::Limiter>,
}

pub struct ApiError(StatusCode, String);
pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn parts(self) -> (StatusCode, String) {
        (self.0, self.1)
    }

    pub fn new(code: StatusCode, msg: impl Into<String>) -> Self {
        Self(code, msg.into())
    }
}
impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => Self(StatusCode::NOT_FOUND, "not found".into()),
            e if e
                .as_database_error()
                .is_some_and(|d| d.is_unique_violation()) =>
            {
                Self(StatusCode::CONFLICT, "already exists".into())
            }
            e => Self(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/stations", get(list_stations).post(create_station))
        .route(
            "/api/stations/{id}/media",
            get(media::list)
                .post(media::upload)
                .layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route("/api/media/{id}", delete(media::remove))
        .route("/api/media/{id}/art", get(media::art))
        .route(
            "/api/stations/{id}/playlists",
            get(playlists::list).post(playlists::create),
        )
        .route(
            "/api/playlists/{id}",
            axum::routing::put(playlists::update).delete(playlists::remove),
        )
        .route(
            "/api/playlists/{id}/media",
            get(playlists::list_media).post(playlists::add_media),
        )
        .route(
            "/api/playlists/{id}/media/{media_id}",
            delete(playlists::remove_media),
        )
        .route(
            "/api/playlists/{id}/schedules",
            get(playlists::list_schedules).post(playlists::add_schedule),
        )
        .route("/api/schedules/{id}", delete(playlists::remove_schedule))
        .route("/api/stations/{id}/djs", get(djs::list).post(djs::create))
        .route(
            "/api/djs/{id}",
            axum::routing::put(djs::update).delete(djs::remove),
        )
        .route("/api/stations/{id}/nowplaying", get(nowplaying::sse))
        .route("/api/stations/{id}/nowplaying.json", get(nowplaying::json))
        .route("/api/stations/{id}/history", get(nowplaying::history))
        .route("/api/stations/{id}/requests", get(requests::pending).post(requests::create))
        .route("/api/stations/{id}/requests/songs", get(requests::songs))
        .route("/api/requests/{id}", delete(requests::cancel))
        .route("/api/station/{station}/requests", get(requests::az_list))
        .route("/api/station/{station}/request/{media_id}", post(requests::az_request).get(requests::az_request))
        .route("/api/stations/{id}/skip", post(skip))
        .route("/api/stations/{id}/previous", post(previous))
        .route("/api/nowplaying", get(azuracast::all))
        .route("/api/nowplaying/{station}", get(azuracast::one))
        .route("/api/nowplaying_static/{station}", get(azuracast::one))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_admin))
        .with_state(state)
}

/// Reads are public (player needs them), except DJ accounts; every write needs Basic Auth `admin:<RUSTCAST_ADMIN_PASSWORD>`.
async fn require_admin(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let auth = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let path = req.uri().path();
    // listener requests are the only public writes (rate-limited in requests.rs)
    let public_write = req.method() == Method::POST
        && ((path.starts_with("/api/stations/") && path.ends_with("/requests"))
            || (path.starts_with("/api/station/") && path.contains("/request/")));
    let private = (req.method() != Method::GET && !public_write) || path.ends_with("/djs");
    if private && !check_basic(auth, &s.admin_password) {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Basic realm=\"rustcast\"")],
        )
            .into_response();
    }
    next.run(req).await
}

async fn station_mount(
    s: &AppState,
    id: i64,
) -> ApiResult<std::sync::Arc<crate::relay::mountpoint::MountPoint>> {
    let st: Station = sqlx::query_as(sql!("SELECT {STATION_COLS} FROM stations WHERE id = ?"))
        .bind(id)
        .fetch_one(&s.db)
        .await?;
    Ok(s.mounts.get_or_create(&st.mountpoint))
}

/// POST /api/stations/{id}/skip — next track (short crossfade). 409 while a live DJ is on.
async fn skip(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> ApiResult<StatusCode> {
    if station_mount(&s, id).await?.request_skip() {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(StatusCode::CONFLICT, "live DJ on air"))
    }
}

/// POST /api/stations/{id}/previous — replay the previous track. 409 if live or no history yet.
async fn previous(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> ApiResult<StatusCode> {
    if station_mount(&s, id).await?.request_previous() {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(
            StatusCode::CONFLICT,
            "live DJ on air, or no previous track",
        ))
    }
}

async fn list_stations(State(s): State<AppState>) -> ApiResult<Json<Vec<Station>>> {
    Ok(Json(
        sqlx::query_as(sql!("SELECT {STATION_COLS} FROM stations ORDER BY id"))
            .fetch_all(&s.db)
            .await?,
    ))
}

#[derive(Deserialize)]
struct NewStation {
    name: String,
    mountpoint: String,
    bitrate: Option<i64>,
    crossfade_secs: Option<f64>,
}

pub fn valid_mount(m: &str) -> bool {
    !m.is_empty()
        && m.len() <= 64
        && m.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

async fn create_station(
    State(s): State<AppState>,
    Json(n): Json<NewStation>,
) -> ApiResult<(StatusCode, Json<Station>)> {
    if !valid_mount(&n.mountpoint) || n.name.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "name required; mountpoint: [A-Za-z0-9_-]{1,64}",
        ));
    }
    let st: Station = sqlx::query_as(sql!(
        "INSERT INTO stations (name, mountpoint, bitrate, crossfade_secs) VALUES (?, ?, ?, ?) RETURNING {STATION_COLS}"
    ))
    .bind(n.name.trim())
    .bind(&n.mountpoint)
    .bind(n.bitrate.unwrap_or(128).clamp(64, 320))
    .bind(n.crossfade_secs.unwrap_or(4.0).clamp(0.0, 15.0))
    .fetch_one(&s.db)
    .await?;
    start_autodj(s.db.clone(), &s.mounts, &st);
    Ok((StatusCode::CREATED, Json(st)))
}
