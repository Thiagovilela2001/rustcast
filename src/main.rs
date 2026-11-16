/// Dynamic SQL built only from compile-time column constants, never user input (values go through `.bind`).
macro_rules! sql {
    ($($t:tt)*) => { sqlx::AssertSqlSafe(format!($($t)*)) };
}

mod api;
mod audio;
mod config;
mod db;
mod relay;
mod web;

use db::models::{STATION_COLS, Station};
use hyper_util::rt::TokioIo;
use relay::{RelayState, mountpoint::Mounts, source_handler};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config::Config::from_env();
    // Few workers is plenty for a radio; each extra thread is stack + scheduler overhead.
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(cfg.threads)
        .max_blocking_threads(4)
        .enable_all()
        .build()?
        .block_on(run(cfg))
}

/// Lets a website on another origin read now playing / API (GET only, so no preflight needed).
async fn add_cors(
    axum::extract::State(origin): axum::extract::State<axum::http::HeaderValue>,
    mut res: axum::response::Response,
) -> axum::response::Response {
    res.headers_mut()
        .insert(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    res
}

async fn run(cfg: config::Config) -> Result<(), Box<dyn std::error::Error>> {
    for (name, v, default) in [
        ("RUSTCAST_SOURCE_PASSWORD", &cfg.source_password, "hackme"),
        ("RUSTCAST_ADMIN_PASSWORD", &cfg.admin_password, "admin"),
    ] {
        if v == default {
            eprintln!(
                "WARNING: {name} is the default '{default}' — set it before exposing this server"
            );
        }
    }
    db::scheduler::UTC_OFFSET_MIN.store(cfg.utc_offset_min, std::sync::atomic::Ordering::Relaxed);
    let pool = db::connect(&cfg.db_path).await?;
    let mounts = Mounts::default();

    sqlx::query("INSERT INTO stations (name, mountpoint) SELECT 'RustCast', 'radio' WHERE NOT EXISTS (SELECT 1 FROM stations)")
        .execute(&pool)
        .await?;
    let stations: Vec<Station> = sqlx::query_as(sql!("SELECT {STATION_COLS} FROM stations"))
        .fetch_all(&pool)
        .await?;
    audio::analyze::start_worker(pool.clone(), cfg.target_lufs);
    for st in &stations {
        db::scheduler::start_autodj(pool.clone(), &mounts, st);
    }

    let relay_state = RelayState {
        mounts: mounts.clone(),
        db: pool.clone(),
        password: cfg.source_password.clone(),
    };
    let app = relay::router(relay_state.clone())
        .merge(api::router(api::AppState {
            db: pool,
            mounts,
            media_dir: cfg.media_dir.into(),
            admin_password: cfg.admin_password,
            public_url: cfg.public_url,
            requests_enabled: cfg.requests_enabled,
            request_cooldown_secs: cfg.request_cooldown_min * 60,
            limiter: Default::default(),
            sessions: Default::default(),
        }))
        .fallback(web::serve);
    let app = match cfg.cors_origin {
        Some(o) => app.layer(axum::middleware::map_response_with_state(
            axum::http::HeaderValue::from_str(&o)?,
            add_cors,
        )),
        None => app,
    };
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    println!(
        "rustcast listening on {} ({} station(s))",
        cfg.bind,
        stations.len()
    );

    // Own accept loop: Icecast `SOURCE`/`PUT` sources go raw, everything else to hyper/axum.
    loop {
        let (sock, peer) = listener.accept().await?;
        let (app, state) = (app.clone(), relay_state.clone());
        tokio::spawn(async move {
            if source_handler::is_source(&sock).await {
                let _ = source_handler::handle_source(sock, state.mounts, state.db, state.password)
                    .await;
            } else {
                // hand the peer IP to handlers (listener requests rate limit)
                let svc = hyper::service::service_fn(move |mut req: hyper::Request<hyper::body::Incoming>| {
                    req.extensions_mut().insert(api::requests::Peer(peer.ip()));
                    tower::ServiceExt::oneshot(app.clone(), req)
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(sock), svc)
                    .await;
            }
        });
    }
}
