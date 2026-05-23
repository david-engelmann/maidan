//! Maidan server entrypoint. Loads config, applies migrations, wires the
//! axum router, and binds to the configured address.

use std::sync::Arc;

use anyhow::Context;
use maidan_artifacts::LocalFsStore;
use maidan_bus::{EventBus, InMemoryBus, PostgresBus};
use maidan_search::{Indexer, LoggingHandler, PostgresSearch, Search, SqliteSearch};
use maidan_server::{config::ArtifactBackend, router, version, AppState, Config};
use maidan_store::{
    run_postgres_migrations, run_sqlite_migrations, Dialect, PostgresStore, SqliteStore, Store,
};
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};

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

    let dialect = Dialect::from_url(&config.database_url).context("detect dialect")?;
    tracing::info!(?dialect, "database dialect");

    let (store, bus, search): (Arc<dyn Store>, Arc<dyn EventBus>, Arc<dyn Search>) = match dialect {
        Dialect::Postgres => {
            let pool = PgPoolOptions::new()
                .max_connections(16)
                .connect(&config.database_url)
                .await
                .context("connect to postgres")?;
            run_postgres_migrations(&pool)
                .await
                .context("apply postgres migrations")?;
            let bus = PostgresBus::connect(pool.clone())
                .await
                .context("connect postgres bus")?;
            tracing::info!("event bus: postgres LISTEN/NOTIFY");
            tracing::info!("search: postgres tsvector");
            (
                Arc::new(PostgresStore::new(pool.clone())),
                Arc::new(bus),
                Arc::new(PostgresSearch::new(pool)),
            )
        }
        Dialect::Sqlite => {
            let pool = SqlitePoolOptions::new()
                .max_connections(8)
                .connect(&config.database_url)
                .await
                .context("connect to sqlite")?;
            run_sqlite_migrations(&pool)
                .await
                .context("apply sqlite migrations")?;
            tracing::info!("event bus: in-memory");
            tracing::info!("search: sqlite fts5");
            (
                Arc::new(SqliteStore::new(pool.clone())),
                Arc::new(InMemoryBus::new()),
                Arc::new(SqliteSearch::new(pool)),
            )
        }
    };

    let artifacts: Arc<dyn maidan_artifacts::ArtifactStore> = match &config.artifact_backend {
        ArtifactBackend::LocalFs { root } => {
            tracing::info!(root = %root.display(), "artifact backend: localfs");
            Arc::new(LocalFsStore::new(root.clone()))
        }
    };

    let state = AppState::new(store, artifacts, bus.clone(), search);
    let app = router(state);

    // Background indexer subscribes to MessagePosted / MessageTombstoned.
    // The default handler logs; future clusters swap in real embedding
    // generators without touching the server wiring.
    let indexer_handler = Arc::new(LoggingHandler::default());
    let indexer = Indexer::new(bus, indexer_handler).spawn();
    tracing::info!("background indexer running");

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("bind {}", config.bind))?;
    tracing::info!(addr = %listener.local_addr()?, "listening");
    let serve_result = axum::serve(listener, app).await.context("axum serve");

    indexer.shutdown().await;
    serve_result?;
    Ok(())
}
