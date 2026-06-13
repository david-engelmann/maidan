//! Maidan server entrypoint. Loads config, applies migrations, wires the
//! axum router, and binds to the configured address.

use std::sync::{atomic::AtomicI64, Arc};

use anyhow::Context;
use maidan_artifacts::{LocalFsStore, S3Config, S3Store};
use maidan_bus::{
    EventBus, InMemoryBus, InMemoryResourceNotifier, PostgresBus, PostgresBusOptions,
    PostgresPresenceNotifier, PostgresResourceNotifier, PresenceNotifier, ResourceNotifier,
};
use maidan_search::{
    EmbeddingHandler, Indexer, LoggingHandler, PostgresSearch, Search, SqliteSearch,
};
use maidan_server::{config::ArtifactBackend, router, version, AppState, Config};
use maidan_store::{
    run_postgres_migrations, run_sqlite_migrations, Dialect, OutboxBackend, PostgresStore,
    SqliteStore, Store,
};
use sqlx::postgres::PgPoolOptions;
use tokio::sync::RwLock;

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

    maidan_server::metrics::init();

    let dialect = Dialect::from_url(&config.database_url).context("detect dialect")?;
    tracing::info!(?dialect, "database dialect");

    let store: Arc<dyn Store>;
    let bus: Arc<dyn EventBus>;
    let resource_notifier: Arc<dyn ResourceNotifier>;
    // Cross-replica presence only applies in Postgres NOTIFY mode; single-process
    // (SQLite / polled relay) keeps the legacy local-only hub (no heartbeat).
    let presence_notifier: Option<Arc<dyn PresenceNotifier>>;
    let search: Arc<dyn Search>;
    let use_embedding_indexer: bool;
    let bus_listener_health: Option<Arc<maidan_bus::ListenerHealth>>;
    let bus_hydrate_stats: Option<Arc<maidan_bus::HydrateStats>>;
    let outbox_relay_enabled = maidan_server::outbox_relay::relay_enabled_from_env();
    let outbox_relay_mode = maidan_server::outbox_relay::relay_mode_from_env();
    maidan_server::outbox_relay::validate_startup(
        maidan_server::config::is_production(),
        outbox_relay_enabled,
    )
    .map_err(anyhow::Error::msg)?;

    let outbox_relay;
    let outbox_backend: Option<OutboxBackend>;

    match dialect {
        Dialect::Postgres => {
            let mut pg_opts = PgPoolOptions::new()
                .max_connections(config.db.max_connections.unwrap_or(16))
                .acquire_timeout(std::time::Duration::from_secs(
                    config.db.acquire_timeout_secs,
                ));
            let statement_timeout_ms = config.db.statement_timeout_ms;
            if statement_timeout_ms > 0 {
                // Cap every pooled connection's queries. Boot migrations exempt
                // their own connection (they reset statement_timeout under the
                // advisory lock); large reindexes should use the CLI's own pool.
                pg_opts = pg_opts.after_connect(move |conn, _meta| {
                    Box::pin(async move {
                        sqlx::query(&format!("SET statement_timeout = {statement_timeout_ms}"))
                            .execute(conn)
                            .await?;
                        Ok(())
                    })
                });
            }
            let pool = pg_opts
                .connect(&config.database_url)
                .await
                .context("connect to postgres")?;
            run_postgres_migrations(&pool)
                .await
                .context("apply postgres migrations")?;
            let notify_on_publish =
                outbox_relay_mode == maidan_server::outbox_relay::OutboxRelayMode::Notify;
            let pg_bus =
                PostgresBus::connect_with(pool.clone(), PostgresBusOptions { notify_on_publish })
                    .await
                    .context("connect postgres bus")?;
            bus_listener_health = Some(pg_bus.listener_health());
            bus_hydrate_stats = Some(pg_bus.hydrate_stats());
            if notify_on_publish {
                tracing::info!("event bus: postgres LISTEN/NOTIFY");
            } else {
                tracing::warn!(
                    "event bus: postgres polled relay mode (pg_notify disabled; single-process fan-out)"
                );
            }
            tracing::info!("search: postgres tsvector");
            outbox_backend = Some(OutboxBackend::Postgres(pool.clone()));
            outbox_relay = outbox_relay_enabled;
            store = Arc::new(PostgresStore::new(pool.clone()));
            bus = Arc::new(pg_bus);
            resource_notifier = if notify_on_publish {
                Arc::new(
                    PostgresResourceNotifier::connect(pool.clone())
                        .await
                        .context("connect postgres resource notifier")?,
                )
            } else {
                // NOTIFY disabled (polled relay): cross-process resource fan-out
                // is unavailable, so fall back to single-process local delivery.
                Arc::new(InMemoryResourceNotifier::new())
            };
            presence_notifier = if notify_on_publish {
                Some(Arc::new(
                    PostgresPresenceNotifier::connect(pool.clone())
                        .await
                        .context("connect postgres presence notifier")?,
                ))
            } else {
                None
            };
            search = Arc::new(PostgresSearch::new(pool));
            use_embedding_indexer = true;
        }
        Dialect::Sqlite => {
            let pool = maidan_search::sqlite_pool_options()
                .max_connections(config.db.max_connections.unwrap_or(8))
                .acquire_timeout(std::time::Duration::from_secs(
                    config.db.acquire_timeout_secs,
                ))
                .connect(&config.database_url)
                .await
                .context("connect to sqlite")?;
            maidan_store::configure_sqlite_pool_with(&pool, config.db.busy_timeout_ms)
                .await
                .context("configure sqlite pragmas")?;
            run_sqlite_migrations(&pool)
                .await
                .context("apply sqlite migrations")?;
            tracing::info!("event bus: in-memory");
            tracing::info!("search: sqlite fts5");
            outbox_backend = Some(OutboxBackend::Sqlite(pool.clone()));
            outbox_relay = outbox_relay_enabled;
            store = Arc::new(SqliteStore::new(pool.clone()));
            bus = Arc::new(InMemoryBus::new());
            resource_notifier = Arc::new(InMemoryResourceNotifier::new());
            presence_notifier = None; // single process: legacy local-only presence
            search = Arc::new(SqliteSearch::new(pool));
            use_embedding_indexer = false;
            bus_listener_health = None;
            bus_hydrate_stats = None;
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

    let embedding_provider =
        maidan_search::provider_from_env().context("MAIDAN_EMBEDDING_PROVIDER")?;
    tracing::info!(
        model = embedding_provider.model_name(),
        dim = embedding_provider.dimension(),
        "embedding provider configured"
    );

    let indexer_last_error = Arc::new(RwLock::new(None));

    let indexer_handler: Arc<dyn maidan_search::EventHandler> = if use_embedding_indexer {
        tracing::info!("indexer: embedding generation (postgres)");
        Arc::new(
            EmbeddingHandler::new(store.clone(), search.clone(), embedding_provider.clone())
                .with_health_error_slot(indexer_last_error.clone()),
        )
    } else {
        tracing::info!("indexer: logging only (sqlite)");
        Arc::new(LoggingHandler::default())
    };

    let auth_disabled = maidan_server::auth::auth_disabled_from_env();
    if auth_disabled {
        tracing::warn!("AUTH_DISABLED is set; bearer tokens are not required");
    }
    #[cfg(feature = "bootstrap")]
    let bootstrap_enabled = maidan_server::bootstrap::bootstrap_enabled_from_env();
    #[cfg(not(feature = "bootstrap"))]
    let bootstrap_enabled = false;
    #[cfg(feature = "bootstrap")]
    if bootstrap_enabled {
        tracing::warn!("MAIDAN_BOOTSTRAP is set; unauthenticated bootstrap routes are enabled");
    }
    let federation_disabled = maidan_server::federation::federation_disabled_from_env();
    if federation_disabled {
        tracing::warn!("FEDERATION_DISABLED is set; federation worker not started");
    }
    let indexer_heartbeat = Arc::new(AtomicI64::new(0));
    let federation_encryption_key = match maidan_auth::encryption_key_from_env() {
        Ok(key) => Some(Arc::new(key)),
        Err(_) => {
            if !federation_disabled {
                tracing::warn!(
                    "FEDERATION_ENCRYPTION_KEY not set; cannot create peers or decrypt outbound secrets after restart"
                );
            }
            None
        }
    };
    let federation = maidan_server::FederationRuntime::new(
        federation_disabled,
        federation_encryption_key.clone(),
    );
    let oidc_runtime =
        match maidan_server::oidc::OidcSettings::from_env().map_err(anyhow::Error::from)? {
            Some(settings) => {
                tracing::info!(mock = settings.mock, "OIDC login enabled");
                Some(
                    maidan_server::oidc::OidcRuntime::init(settings)
                        .await
                        .map_err(anyhow::Error::from)?,
                )
            }
            None => None,
        };
    let subscribe_resume_secret = if oidc_runtime.is_some() {
        None
    } else {
        match maidan_server::subscribe_resume::secret_from_env() {
            Ok(s) => Some(s),
            Err(err) if auth_disabled => {
                tracing::warn!(
                    %err,
                    "subscribe resume uses built-in test secret; set MAIDAN_SESSION_SECRET for production"
                );
                Some(Arc::from(
                    maidan_server::subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET,
                ))
            }
            Err(err) => return Err(err.into()),
        }
    };
    let subscribe_resume_ttl_secs = maidan_server::subscribe_resume::ttl_secs_from_env();
    let mut state = AppState::new(
        store,
        artifacts,
        bus.clone(),
        search,
        embedding_provider,
        auth_disabled,
        bootstrap_enabled,
        federation,
        indexer_heartbeat.clone(),
        bus_listener_health,
    );
    // Cluster 102: MCP resource-update notifications fan out across replicas.
    state.attach_resource_notifier(resource_notifier);
    state.mcp.spawn_resource_notify_listener();
    // Cluster 103: presence/typing/roster fan out across replicas (Postgres NOTIFY).
    if let Some(presence_notifier) = presence_notifier {
        state.attach_presence_notifier(presence_notifier);
    }
    state.presence.spawn_tasks();
    state.indexer_last_error = indexer_last_error;
    state.bus_hydrate_stats = bus_hydrate_stats;
    state.outbox_relay = outbox_relay;
    state.outbox_backend = outbox_backend.clone();
    // Capacity-1 enqueue nudge: `publish` pings the relay so it wakes from idle
    // backoff promptly (Cluster 108.0.2). Only wired when the relay runs.
    let outbox_nudge_rx = if outbox_relay {
        let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
        state.outbox_nudge = Some(tx);
        Some(rx)
    } else {
        None
    };
    state.oidc = oidc_runtime.map(Arc::new);
    state.subscribe_resume_secret = subscribe_resume_secret;
    state.subscribe_resume_ttl_secs = subscribe_resume_ttl_secs;
    state.webhooks = maidan_server::WebhookRuntime::new(federation_encryption_key.clone());
    state.slash = maidan_server::SlashRuntime::new(federation_encryption_key.clone());
    state.fsm_hooks = maidan_server::FsmHookRuntime::new(federation_encryption_key);
    state.rate_limit_redis = maidan_server::rate_limit::connect_redis_from_env().await;
    let app = router(state.clone());

    if outbox_relay {
        if let Some(backend) = outbox_backend {
            let relay_bus = bus.clone();
            let max_attempts = maidan_server::outbox_relay::max_attempts_from_env();
            let poll_interval = maidan_server::outbox_relay::poll_interval_from_env();
            tokio::spawn(async move {
                let mut relay = maidan_server::outbox_relay::OutboxRelay::with_options(
                    backend,
                    relay_bus,
                    max_attempts,
                    poll_interval,
                );
                if let Some(rx) = outbox_nudge_rx {
                    relay = relay.with_nudge(rx);
                }
                relay.run().await;
            });
            tracing::info!(
                mode = outbox_relay_mode.as_str(),
                max_attempts,
                poll_ms = poll_interval.as_millis(),
                "outbox relay running"
            );
        }
    } else {
        tracing::warn!("outbox relay disabled; HTTP handlers publish directly to the bus");
    }

    let indexer = Indexer::new(bus, indexer_handler).spawn_with_heartbeat(indexer_heartbeat);
    tracing::info!("background indexer running");

    if let Err(err) = maidan_server::webhooks::hydrate_webhook_secrets(&state).await {
        tracing::warn!(error = %err, "webhook secret hydration failed");
    }
    let webhook_worker = maidan_server::webhook_worker::WebhookWorker::spawn(state.clone());
    tracing::info!(
        max_attempts = maidan_server::webhooks::max_attempts_from_env(),
        poll_ms = maidan_server::webhooks::poll_interval_ms_from_env(),
        "webhook worker running"
    );

    let fsm_hook_worker = maidan_server::fsm_hook_worker::FsmHookWorker::spawn(state.clone());
    tracing::info!("fsm hook worker running");

    let automation_worker =
        maidan_server::automation_worker::AutomationDeliveryWorker::spawn(state.clone());
    tracing::info!(
        max_attempts = maidan_server::automation_delivery::max_attempts_from_env(),
        poll_ms = maidan_server::automation_delivery::poll_interval_ms_from_env(),
        "automation delivery worker running"
    );

    let federation_worker = if state.federation.disabled {
        None
    } else {
        if let Err(err) = maidan_server::federation::hydrate_federation_secrets(&state).await {
            tracing::warn!(error = %err, "federation peer secret hydration failed");
        }
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
    webhook_worker.shutdown().await;
    automation_worker.shutdown().await;
    fsm_hook_worker.shutdown().await;
    if let Some(worker) = federation_worker {
        worker.shutdown().await;
    }
    serve_result?;
    obs_guard.shutdown();
    Ok(())
}
