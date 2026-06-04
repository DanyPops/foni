pub mod config;
pub mod expression;
mod metrics;
pub mod quality;
pub mod state;
pub mod synth;
pub mod wav;

use axum::{
    routing::{get, post},
    Router,
};

pub async fn build_router_with(cfg: config::ResolvedConfig) -> Router {
    build_router_from_state(state::AppState::from_config(cfg))
}

pub async fn build_router() -> Router {
    build_router_with(config::ServerConfig::load()).await
}

fn build_router_from_state(state: state::AppState) -> Router {
    Router::new()
        .route("/models", get(synth::models_route::list))
        .route("/models/:name", post(synth::models_route::load))
        .route(
            "/params",
            get(synth::params_route::get_params).post(synth::params_route::set_params),
        )
        .route(
            "/ssml-params",
            get(expression::ssml_params_route::get_ssml_params),
        )
        .route("/metrics", get(metrics::metrics))
        .route("/synthesize", post(synth::synthesize_route::synthesize))
        .route("/analyse", post(quality::analyse_route::analyse))
        .route("/process", post(synth::process_route::process))
        .route("/breath", post(expression::breath_route::breath))
        .route(
            "/controller",
            get(quality::controller_route::get_controller)
                .post(quality::controller_route::set_controller),
        )
        .with_state(state)
}
