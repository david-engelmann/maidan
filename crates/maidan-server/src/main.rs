//! Maidan server entrypoint. Loads config, applies migrations, wires the
//! axum router, and binds to the configured address.

use std::sync::Arc;

use anyhow::Context;
use maidan_artifacts::LocalFsStore;
use maidan_server::{config::ArtifactBackend, router, version, AppState, Config};
use maidan_store::{run_postgres_migrations, PostgresStore};
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env().context("load config from env")?;

    tracing_subscriber::fmt()
        .with_env_filter(&config.log_filter)
        .with_target(false)
        .init();

    tracing::info!(
        version = version(),
        bind = %config.bind,
        "maidan-server starting"
    );

    let pool = PgPoolOptions::new()
        .max_connections(16)
        .connect(&config.database_url)
        .await
        .context("connect to database")?;
    run_postgres_migrations(&pool)
        .await
        .context("apply database migrations")?;
    let store = Arc::new(PostgresStore::new(pool));

    let artifacts: Arc<dyn maidan_artifacts::ArtifactStore> = match &config.artifact_backend {
        ArtifactBackend::LocalFs { root } => {
            tracing::info!(root = %root.display(), "artifact backend: localfs");
            Arc::new(LocalFsStore::new(root.clone()))
        }
    };

    let state = AppState::new(store, artifacts);
    let app = router(state);

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("bind {}", config.bind))?;
    tracing::info!(addr = %listener.local_addr()?, "listening");
    axum::serve(listener, app).await.context("axum serve")?;

    Ok(())
}
