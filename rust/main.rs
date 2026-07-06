use std::net::SocketAddr;

use anyhow::Result;
use tailgate::{
    app::{build_router, AppState},
    config::load_config,
};
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = load_config()?;
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    let client = reqwest::Client::builder()
        .user_agent("tailgate-rs/0.1.0")
        .build()?;
    let state = AppState::new(config, client);
    state.start_background_tasks();
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(host = %addr.ip(), port = addr.port(), "tailgate listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn init_tracing() {
    let level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).json().init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
