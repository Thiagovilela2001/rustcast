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

use db::models::{Station, STATION_COLS};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use relay::{mountpoint::Mounts, source_handler, RelayState};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config::Config::from_env();
    for (name, v, default) in [("RUSTCAST_SOURCE_PASSWORD", &cfg.source_password, "hackme"), ("RUSTCAST_ADMIN_PASSWORD", &cfg.admin_password, "admin")] {
        if v == default {
            eprintln!("WARNING: {name} is the default '{default}' — set it before exposing this server");
        }
    }
    let pool = db::connect(&cfg.db_path).await?;
    let mounts = Mounts::default();

    sqlx::query("INSERT INTO stations (name, mountpoint) SELECT 'RustCast', 'radio' WHERE NOT EXISTS (SELECT 1 FROM stations)")
        .execute(&pool)
        .await?;
    let stations: Vec<Station> = sqlx::query_as(sql!("SELECT {STATION_COLS} FROM stations")).fetch_all(&pool).await?;
    for st in &stations {
        db::scheduler::start_autodj(pool.clone(), &mounts, st);
    }

    let relay_state = RelayState { mounts: mounts.clone(), db: pool.clone(), password: cfg.source_password.clone() };
    let app = relay::router(relay_state.clone()).merge(api::router(api::AppState {
        db: pool,
        mounts,
        media_dir: cfg.media_dir.into(),
        admin_password: cfg.admin_password,
    }))
    .fallback(web::serve);
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    println!("rustcast listening on {} ({} station(s))", cfg.bind, stations.len());

    // Own accept loop: Icecast `SOURCE`/`PUT` sources go raw, everything else to hyper/axum.
    loop {
        let (sock, _) = listener.accept().await?;
        let (app, state) = (app.clone(), relay_state.clone());
        tokio::spawn(async move {
            if source_handler::is_source(&sock).await {
                let _ = source_handler::handle_source(sock, state.mounts, state.db, state.password).await;
            } else {
                let svc = TowerToHyperService::new(app);
                let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(sock), svc).await;
            }
        });
    }
}
