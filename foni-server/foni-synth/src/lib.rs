/// foni-synth public library surface — exposed for integration tests.
pub mod config;
pub mod dsp;
pub mod routes;
pub mod ssml;
pub mod state;
pub mod wav;

use axum::{Router, routing::{get, post}};

/// Build the production router. Shared by main() and integration tests.
pub async fn build_router() -> Router {
    let state = state::AppState::load().await.expect("failed to load state");
    Router::new()
        .route("/models",       get(routes::models::list))
        .route("/models/:name", post(routes::models::load))
        .route("/params",       get(routes::params::get_params).post(routes::params::set_params))
        .route("/convert",      post(routes::convert::convert))
        .route("/analyse",      post(routes::analyse::analyse))
        .route("/process",      post(routes::process::process))
        .route("/breath",       post(routes::breath::breath))
        .with_state(state)
}
