use std::time::Duration;

use ax_api::{AppState, Config, router};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let config = Config::from_env()?;
    let db = match &config.database_url {
        Some(url) => {
            Some(PgPoolOptions::new().max_connections(10).acquire_timeout(Duration::from_secs(3)).connect_lazy(url)?)
        }
        None => {
            tracing::warn!("DATABASE_URL is not set; running without a database");
            None
        }
    };

    let app = router(AppState { db }, &config);
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(
        addr = %config.bind,
        base_path = %config.base_path,
        contract = ax_contract::source_commit(),
        "listening"
    );
    axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await?;
    Ok(())
}

/// Ctrl-C, or SIGTERM from `docker stop`.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutting down");
}
