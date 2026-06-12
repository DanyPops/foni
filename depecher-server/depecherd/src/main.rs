use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cfg = depecherd::config::ServerConfig::load();
    let addr = cfg.addr.clone();
    let app = depecherd::build_router_with(cfg).await;

    tracing::info!("depecherd listening on {addr}");
    // Brief retry loop — systemd may restart before the OS reclaims the port.
    let listener = {
        let mut last_err = String::new();
        let mut result = None;
        for attempt in 0..5u32 {
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => {
                    result = Some(l);
                    break;
                }
                Err(e) => {
                    last_err = e.to_string();
                    tokio::time::sleep(std::time::Duration::from_millis(500 * 2u64.pow(attempt)))
                        .await;
                }
            }
        }
        result.unwrap_or_else(|| {
            tracing::error!("cannot bind {addr}: {last_err}");
            std::process::exit(1)
        })
    };
    axum::serve(listener, app).await.unwrap();
}
