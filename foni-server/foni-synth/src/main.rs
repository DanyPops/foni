use axum::{Router, routing::{get, post}};
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;

mod config;
pub mod dsp;
mod routes;
pub mod ssml;
mod state;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let state = state::AppState::load().await.expect("failed to load state");

    let app = Router::new()
        .route("/models",         get(routes::models::list))
        .route("/models/:name",   post(routes::models::load))
        .route("/params",         get(routes::params::get_params).post(routes::params::set_params))
        .route("/convert",        post(routes::convert::convert))
        .route("/analyse",        post(routes::analyse::analyse))
        .with_state(state);

    let addr: SocketAddr = "0.0.0.0:5050".parse().unwrap();
    tracing::info!("foni-synth listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
