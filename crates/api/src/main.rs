use std::process::ExitCode;
use std::time::Duration;

use ax_api::attachments::Files;
use ax_api::cli::{Cli, Command};
use ax_api::{AppState, Config, db, router};
use clap::Parser;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

type BoxError = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    // PostgreSQL NOTICEs (e.g. "relation _sqlx_migrations already exists,
    // skipping" on every migration run) are noise at info level.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("sqlx::postgres::notice=warn".parse().expect("a valid directive"));
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();

    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), BoxError> {
    let config = Config::from_env()?;
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve(config).await,
        Command::Migrate => {
            db::migrate(&connect(&config)?).await?;
            println!("migrations applied");
            Ok(())
        }
        Command::User(command) => {
            let pool = connect(&config)?;
            db::migrate(&pool).await?;
            ax_api::cli::run_user(&pool, command).await
        }
    }
}

fn connect(config: &Config) -> Result<PgPool, BoxError> {
    let url = config.database_url.as_deref().ok_or("DATABASE_URL is not set")?;
    Ok(PgPoolOptions::new().max_connections(10).acquire_timeout(Duration::from_secs(3)).connect_lazy(url)?)
}

async fn serve(config: Config) -> Result<(), BoxError> {
    let db = match config.database_url {
        Some(_) => {
            let pool = connect(&config)?;
            db::migrate(&pool).await?;
            Some(pool)
        }
        None => {
            tracing::warn!("DATABASE_URL is not set; running without a database");
            None
        }
    };

    if let Some(pool) = &db {
        let known: bool = sqlx::query_scalar("select exists (select 1 from pg_timezone_names where name = $1)")
            .bind(&config.time_zone.0)
            .fetch_one(pool)
            .await?;
        if !known {
            return Err(format!("AX_TIME_ZONE is not a time zone PostgreSQL knows: {:?}", config.time_zone.0).into());
        }
    }
    std::fs::create_dir_all(&config.upload_dir)
        .map_err(|error| format!("AX_UPLOAD_DIR {:?} cannot be used: {error}", config.upload_dir))?;
    let file_key = match &db {
        Some(pool) => ax_api::attachments::signing_key(pool).await?,
        None => ax_api::attachments::random_key(),
    };
    let files = Files::new(&config.upload_dir, &config.public_path, file_key);
    if let Some(pool) = &db {
        // The configuration allows only these two.
        if config.ai.provider == "anthropic" {
            let claude = ax_api::ai::claude::ClaudeProvider::new(config.ai.anthropic.clone());
            ax_api::ai::jobs::spawn(pool.clone(), claude, config.ai.clone());
        } else {
            ax_api::ai::jobs::spawn(pool.clone(), ax_api::ai::StubProvider, config.ai.clone());
        }
        tracing::info!(provider = %config.ai.provider, "AI worker started");
    }
    let state = AppState {
        db,
        sessions: config.sessions,
        setup_links: config.setup_links,
        login: config.login,
        time_zone: config.time_zone.clone(),
        ai: config.ai.clone(),
        drafts: config.drafts,
        files,
        claude: (config.ai.provider == "anthropic")
            .then(|| ax_api::ai::claude::ClaudeProvider::new(config.ai.anthropic.clone())),
    };
    let app = router(state, &config);
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
