//! radar-api: the single-process replacement for the radar-ng tile-server pod.
//!
//! One axum server does the work of three processes: the Caddy front proxy (static lanes, the
//! error override, `X-Client-IP`, the access log), the FastAPI application (every `/api` and `/v1`
//! route), and the go-pmtiles sidecar (basemap vector tiles, read here straight from the archive).
//! Routes, parameters, JSON bodies, status codes and headers match the Python service so the apps
//! do not change. Deliberate differences are listed in the crate README.
//!
//! Public surface: [`router`] builds the application, [`serve`] runs it, and [`AppState::from_env`]
//! reads the process configuration.

mod basemap;
mod briefing;
mod client_ip;
mod config;
mod edge;
mod files;
mod forecast;
mod grid;
mod health;
mod manifest;
mod metrics;
mod nowcast;
mod params;
mod places;
mod prefetch;
mod ratelimit;
mod respond;
mod state;
mod upstream;
mod wind;
mod workflows;

use std::net::SocketAddr;

use anyhow::{Context, Result};
use axum::Router;
use axum::middleware::{from_fn, from_fn_with_state};

pub use config::Config;
pub use state::AppState;

/// The whole application: the `/api` routes, the `/v1` routes (unless `DISABLE_WORKFLOW_ROUTES=1`),
/// the static lanes, and the global middleware.
pub fn router(state: AppState) -> Router {
    let mut api = edge::api_router();
    if state.config.workflow_routes_enabled {
        api = api.merge(workflows::routes());
    }
    let statics = basemap::static_router(
        state.config.tile_dir.clone(),
        state.config.basemap_root.clone(),
    )
    .layer(from_fn(basemap::static_error_override));
    Router::new()
        .merge(api)
        .merge(statics)
        .fallback(edge::unmatched)
        .layer(from_fn_with_state(state.clone(), edge::edge))
        .with_state(state)
}

/// Bind `config.listen_addr` and serve until Ctrl-C or SIGTERM.
///
/// # Errors
/// Fails when logging cannot start, the state cannot be built, or the listener cannot bind.
pub async fn serve(config: Config) -> Result<()> {
    radar_core::logging::init("radar-api")?;
    let addr: SocketAddr = config.listen_addr;
    let state = AppState::new(config)?;
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!("radar-api listening on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serving HTTP")?;
    Ok(())
}

/// Resolves on Ctrl-C, or on SIGTERM where the platform has it.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
