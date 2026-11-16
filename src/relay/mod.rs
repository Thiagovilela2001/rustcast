pub mod icy;
pub mod mountpoint;
pub mod source_handler;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{stream, StreamExt};
use mountpoint::{normalize, Mounts};
use serde::Deserialize;
use std::convert::Infallible;
use tokio::sync::broadcast::error::RecvError;

#[derive(Clone)]
pub struct RelayState {
    pub mounts: Mounts,
    pub db: sqlx::SqlitePool,
    pub password: String,
}

pub fn router(state: RelayState) -> Router {
    Router::new()
        .route("/mount/{name}", get(listen))
        .route("/admin/metadata", get(update_metadata))
        .with_state(state)
}


async fn listen(State(s): State<RelayState>, Path(name): Path<String>, h: HeaderMap) -> Response {
    let Some(mount) = s.mounts.get(&name) else { return StatusCode::NOT_FOUND.into_response() };
    let icy = h.get("icy-metadata").is_some_and(|v| v.as_bytes() == b"1");
    let (burst, rx) = mount.subscribe();
    let live = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(b) => return Some((b, rx)),
                Err(RecvError::Lagged(_)) => continue, // slow listener: drop chunks, keep going
                Err(RecvError::Closed) => return None,
            }
        }
    });
    let raw = stream::iter(burst).chain(live);
    let ctype = mount.content_type.lock().unwrap().clone();
    let mut resp = Response::builder()
        .header(header::CONTENT_TYPE, ctype)
        .header(header::CACHE_CONTROL, "no-cache")
        .header("icy-name", name);
    let body = if icy {
        resp = resp.header("icy-metaint", icy::METAINT.to_string());
        let mut inj = icy::IcyInjector::new(mount.meta.subscribe());
        Body::from_stream(raw.flat_map(move |b| stream::iter(inj.process(b))).map(Ok::<_, Infallible>))
    } else {
        Body::from_stream(raw.map(Ok::<_, Infallible>))
    };
    resp.body(body).unwrap()
}

#[derive(Deserialize)]
struct MetaQ {
    mount: String,
    song: String,
}

/// Icecast-compatible: GET /admin/metadata?mount=/radio&mode=updinfo&song=Artist%20-%20Title
async fn update_metadata(State(s): State<RelayState>, h: HeaderMap, Query(q): Query<MetaQ>) -> StatusCode {
    let name = normalize(&q.mount);
    let auth = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if !source_handler::source_auth(&s.db, &name, auth, &s.password).await {
        return StatusCode::UNAUTHORIZED;
    }
    let Some(mount) = s.mounts.get(&name) else { return StatusCode::NOT_FOUND };
    mount.meta.send_replace(q.song);
    StatusCode::OK
}
