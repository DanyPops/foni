use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cfg = foni_synth::config::ServerConfig::load();
    let addr = cfg.addr.clone();
    let app = foni_synth::build_router_with(cfg).await;

    tracing::info!("foni-synth listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
