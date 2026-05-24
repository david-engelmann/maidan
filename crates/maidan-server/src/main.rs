//! Maidan server entrypoint. Loads config, applies migrations, wires the
//! axum router, and binds to the configured address.

use std::sync::{atomic::AtomicI64, Arc};

use anyhow::Context;
use maidan_artifacts::{LocalFsStore, S3Config, S3Store};
use maidan_bus::{EventBus, InMemoryBus, PostgresBus};
use maidan_search::{
    EmbeddingHandler, Indexer, LoggingHandler, PostgresSearch, Search, SqliteSearch,
};
use maidan_server::{config::ArtifactBackend, router, version, AppState, Config};
use maidan_store::{
    run_postgres_migrations, run_sqlite_migrations, Dialect, PostgresStore, SqliteStore, Store,
};
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env().context("load config from env")?;

    let mut obs_config = maidan_observability::Config::from_env();
    obs_config.log_filter = config.log_filter.clone();
    let obs_guard = maidan_observability::init(obs_config).context("init observability")?;

    tracing::info!(
        version = version(),
        bind = %config.bind,
        "maidan-server starting"
    );

    let dialect = Dialect::from_url(&config.database_url).context("detect dialect")?;
    tracing::info!(?dialect, "database dialect");

    let store: Arc<dyn Store>;
    let bus: Arc<dyn EventBus>;
    let search: Arc<dyn Search>;
    let use_embedding_indexer: bool;
    let bus_listener_health: Option<Arc<maidan_bus::ListenerHealth>>;

    match dialect {
        Dialect::Postgres => {
            let pool = PgPoolOptions::new()
                .max_connections(16)
                .connect(&config.database_url)
                .await
                .context("connect to postgres")?;
            run_postgres_migrations(&pool)
                .await
                .context("apply postgres migrations")?;
            let pg_bus = PostgresBus::connect(pool.clone())
                .await
                .context("connect postgres bus")?;
            bus_listener_health = Some(pg_bus.listener_health());
            tracing::info!("event bus: postgres LISTEN/NOTIFY");
            tracing::info!("search: postgres tsvector");
            store = Arc::new(PostgresStore::new(pool.clone()));
            bus = Arc::new(pg_bus);
            search = Arc::new(PostgresSearch::new(pool));
            use_embedding_indexer = true;
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
            store = Arc::new(SqliteStore::new(pool.clone()));
            bus = Arc::new(InMemoryBus::new());
            search = Arc::new(SqliteSearch::new(pool));
            use_embedding_indexer = false;
            bus_listener_health = None;
        }
    };

    let artifacts: Arc<dyn maidan_artifacts::ArtifactStore> = match &config.artifact_backend {
        ArtifactBackend::LocalFs { root } => {
            tracing::info!(root = %root.display(), "artifact backend: localfs");
            Arc::new(LocalFsStore::new(root.clone()))
        }
        ArtifactBackend::S3 {
            endpoint,
            bucket,
            region,
            access_key,
            secret_key,
        } => {
            tracing::info!(endpoint, bucket, "artifact backend: s3");
            let store = S3Store::new(S3Config {
                endpoint: endpoint.clone(),
                bucket: bucket.clone(),
                region: region.clone(),
                access_key: access_key.clone(),
                secret_key: secret_key.clone(),
            })
            .await
            .context("connect s3 artifact backend")?;
            Arc::new(store)
        }
    };

    let indexer_handler: Arc<dyn maidan_search::EventHandler> = if use_embedding_indexer {
        tracing::info!("indexer: hash-v1 embedding generation (postgres)");
        Arc::new(EmbeddingHandler::new(store.clone(), search.clone()))
    } else {
        tracing::info!("indexer: logging only (sqlite)");
        Arc::new(LoggingHandler::default())
    };

    let auth_disabled = maidan_server::auth::auth_disabled_from_env();
    if auth_disabled {
        tracing::warn!("AUTH_DISABLED is set; bearer tokens are not required");
    }
    let federation_disabled = maidan_server::federation::federation_disabled_from_env();
    if federation_disabled {
        tracing::warn!("FEDERATION_DISABLED is set; federation worker not started");
    }
    let indexer_heartbeat = Arc::new(AtomicI64::new(0));
    let state = AppState::new(
        store,
        artifacts,
        bus.clone(),
        search,
        auth_disabled,
        federation_disabled,
        indexer_heartbeat.clone(),
        bus_listener_health,
    );
    let app = router(state.clone());

    let indexer = Indexer::new(bus, indexer_handler).spawn_with_heartbeat(indexer_heartbeat);
    tracing::info!("background indexer running");

    let federation_worker = if federation_disabled {
        None
    } else {
        tracing::info!(
            secs = maidan_server::federation::poll_interval_secs_from_env(),
            "federation worker running"
        );
        Some(maidan_server::federation_worker::FederationWorker::spawn(
            state.clone(),
        ))
    };

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("bind {}", config.bind))?;
    tracing::info!(addr = %listener.local_addr()?, "listening");

    let shutdown = async {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("shutdown signal received");
        }
    };

    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .context("axum serve");

    indexer.shutdown().await;
    if let Some(worker) = federation_worker {
        worker.shutdown().await;
    }
    serve_result?;
    obs_guard.shutdown();
    Ok(())
}
