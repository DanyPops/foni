/// foni-synth public library surface — exposed for integration tests.
pub mod config;
pub mod dsp;
pub mod routes;
pub mod sessions;
pub mod ssml;
pub mod state;
pub mod wav;

use axum::{
    routing::{get, post},
    Router,
};

/// Build the production router with explicit config. Used by main().
pub async fn build_router_with(cfg: config::ServerConfig) -> Router {
    build_router_from_state(state::AppState::from_config(cfg))
}

/// Build the production router with auto-loaded config. Used by integration tests.
pub async fn build_router() -> Router {
    build_router_with(config::ServerConfig::load()).await
}

fn build_router_from_state(state: state::AppState) -> Router {
    Router::new()
        .route("/models", get(routes::models::list))
        .route("/models/:name", post(routes::models::load))
        .route(
            "/params",
            get(routes::params::get_params).post(routes::params::set_params),
        )
        .route("/ssml-params", get(routes::ssml_params::get_ssml_params))
        .route("/convert", post(routes::convert::convert))
        .route("/synthesize", post(routes::synthesize::synthesize))
        .route("/analyse", post(routes::analyse::analyse))
        .route("/process", post(routes::process::process))
        .route("/breath", post(routes::breath::breath))
        .with_state(state)
}
