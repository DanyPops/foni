use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let app = foni_synth::build_router().await;
    let addr = std::env::var("FONI_SYNTH_ADDR").unwrap_or_else(|_| "0.0.0.0:5050".into());
    tracing::info!("foni-synth listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
