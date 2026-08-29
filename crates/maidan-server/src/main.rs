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
    BatchConfig, BatchingEmbeddingHandler, Indexer, IndexerMetrics, LoggingHandler, PostgresSearch,
    Search, SqliteSearch,
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
    let read_routing_metrics: Option<Arc<maidan_store::postgres::ReadRoutingMetrics>>;
    let search_read_routing_metrics: Option<Arc<maidan_search::SearchReadMetrics>>;
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
            let max_connections = config.db.max_connections.unwrap_or(16);
            let acquire_timeout_secs = config.db.acquire_timeout_secs;
            let statement_timeout_ms = config.db.statement_timeout_ms;
            // Build a pool options with the same connection setup for the primary
            // and (when configured) the read replica. Caps every pooled
            // connection's queries; boot migrations exempt their own connection
            // (they reset statement_timeout under the advisory lock), and large
            // reindexes should use the CLI's own pool.
            let make_pg_opts = || {
                let mut o = PgPoolOptions::new()
                    .max_connections(max_connections)
                    .acquire_timeout(std::time::Duration::from_secs(acquire_timeout_secs));
                if statement_timeout_ms > 0 {
                    o = o.after_connect(move |conn, _meta| {
                        Box::pin(async move {
                            sqlx::query(&format!("SET statement_timeout = {statement_timeout_ms}"))
                                .execute(conn)
                                .await?;
                            Ok(())
                        })
                    });
                }
                o
            };
            let pool = make_pg_opts()
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
            // Read-replica pool (Cluster 262): when MAIDAN_DB_REPLICA_URL is set,
            // connect a separate reader pool (validating replica reachability at
            // boot) so LSN-token read routing (Cluster 264) can send eligible reads
            // there. Unset → reads stay on the primary (unchanged).
            let pg_store = if let Some(replica_url) = config.replica_url.as_deref() {
                let reader = make_pg_opts()
                    .connect(replica_url)
                    .await
                    .context("connect to postgres read replica (MAIDAN_DB_REPLICA_URL)")?;
                tracing::info!("read replica: connected (MAIDAN_DB_REPLICA_URL)");
                PostgresStore::with_replica_reader(pool.clone(), reader)
            } else {
                PostgresStore::new(pool.clone())
            };
            read_routing_metrics = config
                .replica_url
                .is_some()
                .then(|| pg_store.read_routing_metrics());
            store = Arc::new(pg_store);
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
            // Search gets its own replica reader pool (Cluster 271) so search reads
            // honor the same per-request consistency token as store reads. A separate
            // pool from the store's reader (pools are not shared across the Arc).
            let pg_search = match config.replica_url.as_deref() {
                Some(replica_url) => {
                    let search_reader = make_pg_opts().connect(replica_url).await.context(
                        "connect to postgres read replica for search (MAIDAN_DB_REPLICA_URL)",
                    )?;
                    PostgresSearch::with_replica_reader(pool, search_reader)
                }
                None => PostgresSearch::new(pool),
            };
            // Capture the search routing counters for `maidan_search_replica_reads_total`
            // when a replica is configured (Cluster 272); `None` otherwise.
            search_read_routing_metrics = config
                .replica_url
                .is_some()
                .then(|| pg_search.read_routing_metrics());
            search = Arc::new(pg_search);
            use_embedding_indexer = true;
        }
        Dialect::Sqlite => {
            // Per-connection PRAGMAs (foreign_keys, busy_timeout, WAL) are applied
            // in the pool's after_connect (Cluster 166) so every pooled connection
            // enforces them — not just the first.
            let pool = maidan_search::sqlite_pool_options_with(config.db.busy_timeout_ms)
                // Serialize through one connection by default: a multi-connection SQLite
                // pool deadlocks concurrent read-modify-write transactions (Cluster 277,
                // see maidan_store::DEFAULT_SQLITE_MAX_CONNECTIONS). Overridable via
                // MAIDAN_DB_MAX_CONNECTIONS.
                .max_connections(
                    config
                        .db
                        .max_connections
                        .unwrap_or(maidan_store::DEFAULT_SQLITE_MAX_CONNECTIONS),
                )
                .acquire_timeout(std::time::Duration::from_secs(
                    config.db.acquire_timeout_secs,
                ))
                .connect(&config.database_url)
                .await
                .context("connect to sqlite")?;
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
            search_read_routing_metrics = None;
            use_embedding_indexer = false;
            bus_listener_health = None;
            bus_hydrate_stats = None;
            read_routing_metrics = None;
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
    // `hash-v1` is a deterministic hash, not a real embedding — semantic search over it
    // returns near-random results. Warn loudly at boot so a stranger who leaves the
    // provider unset isn't silently served meaningless "semantic" hits (Cluster 315).
    if embedding_provider.model_name() == "hash-v1" {
        tracing::warn!(
            "embedding provider is `hash-v1`: a deterministic hash, NOT semantically \
             meaningful — semantic search will return near-random results. Set \
             MAIDAN_EMBEDDING_PROVIDER=openai-compatible (+ MAIDAN_EMBEDDING_* config) for \
             real semantic search; lexical/full-text search is unaffected."
        );
    }

    // Register the active model in the per-model table scheme at boot (Cluster
    // 117) so it's queryable before the first write and a dimension mismatch
    // surfaces now. Non-fatal: embeddings are best-effort and the per-message
    // path re-attempts; the server must still serve messaging.
    match search.ensure_model(embedding_provider.as_ref()).await {
        Ok(()) => tracing::info!(
            model = embedding_provider.model_name(),
            dimension = embedding_provider.dimension(),
            "embedding model registered"
        ),
        Err(err) => tracing::warn!(
            error = %err,
            model = embedding_provider.model_name(),
            "embedding model registration at startup failed; will retry on first write"
        ),
    }

    let indexer_last_error = Arc::new(RwLock::new(None));

    let (indexer_handler, indexer_metrics): (
        Arc<dyn maidan_search::EventHandler>,
        Arc<IndexerMetrics>,
    ) = if use_embedding_indexer {
        let config = BatchConfig::from_env();
        let metrics = Arc::new(IndexerMetrics::new(config.queue_capacity));
        tracing::info!(
            queue_capacity = config.queue_capacity,
            batch_size = config.batch_size,
            "indexer: batched embedding generation (postgres)"
        );
        let handler = BatchingEmbeddingHandler::spawn(
            store.clone(),
            search.clone(),
            embedding_provider.clone(),
            config,
            metrics.clone(),
            Some(indexer_last_error.clone()),
        );
        (Arc::new(handler), metrics)
    } else {
        tracing::info!("indexer: logging only (sqlite)");
        (
            Arc::new(LoggingHandler::default()),
            Arc::new(IndexerMetrics::default()),
        )
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
    // Key rotation (Cluster 189): old keys from FEDERATION_DECRYPT_KEYS stay
    // available for decrypt after the primary is rotated. A malformed entry is a
    // hard error — silently dropping an old key would strand its ciphertexts.
    let decrypt_fallback_keys =
        maidan_auth::decrypt_fallback_keys_from_env().context("FEDERATION_DECRYPT_KEYS")?;
    if !decrypt_fallback_keys.is_empty() {
        tracing::info!(
            count = decrypt_fallback_keys.len(),
            "secret key rotation: decrypt fallback keys loaded"
        );
    }
    maidan_auth::init_decrypt_fallback_keys(decrypt_fallback_keys);
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
    state.indexer_metrics = indexer_metrics;
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
    // Default-on global rate limit (Cluster 183): a deployment that configures
    // nothing still gets a DoS floor. `MAIDAN_RATE_LIMIT_MAX` (incl. `0`) overrides.
    state.rate_limit_default_on = true;
    // A2A Agent Card transport advertisement (Cluster 288): public origin for
    // absolute interface URLs + the advertised gRPC address (§5.2 negotiation).
    state.a2a_card = maidan_server::a2a_agent::A2aCardConfig::from_env();
    // Read-replica routing (Cluster 263+): when a replica is configured, stamp the
    // consistency token on writes and route replica-eligible reads.
    state.read_replica_enabled = config.replica_url.is_some();
    state.read_routing_metrics = read_routing_metrics;
    state.search_read_routing_metrics = search_read_routing_metrics;

    // Email transport (Cluster 249): wire it only when `MAIDAN_SMTP_*` is
    // configured — otherwise the notification router sends no email.
    if let Some(smtp) = maidan_server::mail::SmtpConfig::from_env() {
        match maidan_server::mail::SmtpTransport::from_config(&smtp) {
            Ok(transport) => {
                state.attach_mail(std::sync::Arc::new(transport));
                tracing::info!(host = %smtp.host, "email transport configured");
            }
            Err(err) => {
                tracing::warn!(error = %err, "email transport config invalid; email disabled");
            }
        }
    }

    // Background mail-outbox worker (Cluster 305): drains the durable mail queue
    // with retry/backoff + dead-lettering. Runs whenever a transport is
    // configured (the router enqueues only then). Tick via
    // `MAIDAN_MAIL_WORKER_TICK_SECS` (default 5s).
    if state.mail.is_some() {
        let mail_state = state.clone();
        let mail_cfg = maidan_server::mail_worker::config_from_env();
        tokio::spawn(async move {
            maidan_server::mail_worker::run(mail_state, mail_cfg).await;
        });
    }

    // Slack projector (Cluster 307): wire the ingress config when
    // `MAIDAN_SLACK_SIGNING_SECRET` is set — otherwise `/integrations/slack/events`
    // stays disabled (404). A projector, not a bot: no LLM in Maidan.
    if let Some(slack_cfg) = maidan_server::slack::SlackConfig::from_env() {
        // Egress (Cluster 309) needs a bot token for chat.postMessage; ingress works
        // without one. The egress runs on the notification-router bus consumer.
        if let Some(bot_token) = slack_cfg.bot_token.clone() {
            state.attach_slack_sender(std::sync::Arc::new(
                maidan_server::slack::SlackWebClient::new(bot_token),
            ));
            tracing::info!("slack projector egress configured");
        }
        state.attach_slack(std::sync::Arc::new(slack_cfg));
        tracing::info!("slack projector ingress configured");
    }

    // Git/GitHub projector (Cluster 310): wire the ingress config when
    // `MAIDAN_GITHUB_WEBHOOK_SECRET` is set — otherwise
    // `/integrations/github/events` stays disabled (404). A projector, not a bot.
    if let Some(github_cfg) = maidan_server::github::GithubConfig::from_env() {
        // Egress (Cluster 312) needs a token for issue-comment posts; ingress works
        // without one. The egress runs on the notification-router bus consumer.
        if let Some(token) = github_cfg.api_token.clone() {
            state.attach_github_sender(std::sync::Arc::new(
                maidan_server::github::GithubApiClient::new(token),
            ));
            tracing::info!("github projector egress configured");
        }
        state.attach_github(std::sync::Arc::new(github_cfg));
        tracing::info!("github projector ingress configured");
    }

    // Background data-retention sweeper (Cluster 186): opt-in via
    // `MAIDAN_RETENTION_*_DAYS`. Prunes the event log (floored at the durable
    // delivery watermark), audit trail, and delivery tables past their age.
    if let Some(retention_cfg) = maidan_server::retention::config_from_env() {
        let retention_store = state.store.clone();
        tokio::spawn(async move {
            maidan_server::retention::run(retention_store, retention_cfg).await;
        });
    }

    // Background scheduled/recurring-task sweeper (Cluster 227): opt-in via
    // `MAIDAN_SCHEDULER_TICK_SECS`. Materializes a task thread for each schedule
    // that comes due.
    if let Some(scheduler_cfg) = maidan_server::scheduler::config_from_env() {
        let scheduler_state = state.clone();
        tokio::spawn(async move {
            maidan_server::scheduler::run(scheduler_state, scheduler_cfg).await;
        });
    }

    // Background email-digest sweeper (Cluster 255): opt-in via
    // `MAIDAN_DIGEST_TICK_SECS`. Emails digest-mode members an unread rollup.
    if let Some(digest_cfg) = maidan_server::digest::config_from_env() {
        let digest_state = state.clone();
        tokio::spawn(async move {
            maidan_server::digest::run(digest_state, digest_cfg).await;
        });
    }

    // A2A gRPC binding (Cluster 287, §10): opt-in via `MAIDAN_A2A_GRPC_ADDR`
    // (e.g. `0.0.0.0:50051`). Serves the tonic A2AService on a separate port; the
    // HTTP surface (REST + JSON-RPC) is unaffected.
    if let Ok(grpc_addr) = std::env::var("MAIDAN_A2A_GRPC_ADDR") {
        match grpc_addr.parse::<std::net::SocketAddr>() {
            Ok(addr) => {
                let grpc_state = state.clone();
                tokio::spawn(async move {
                    if let Err(err) = maidan_server::a2a_grpc::serve(grpc_state, addr).await {
                        tracing::error!(error = %err, "a2a gRPC server exited");
                    }
                });
                tracing::info!(%addr, "a2a gRPC server listening");
            }
            Err(err) => {
                tracing::error!(error = %err, addr = %grpc_addr, "invalid MAIDAN_A2A_GRPC_ADDR")
            }
        }
    }

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

    let notification_router =
        maidan_server::notification_router::NotificationRouter::spawn(state.clone());
    tracing::info!("notification router running");

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

    // Drain on SIGINT (ctrl_c) *and* SIGTERM — Kubernetes/systemd send SIGTERM
    // on rollout/stop, and without handling it the process is killed mid-request
    // instead of draining via `with_graceful_shutdown`.
    let shutdown = async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            match signal(SignalKind::terminate()) {
                Ok(mut sigterm) => {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {
                            tracing::info!("shutdown signal received (SIGINT)");
                        }
                        _ = sigterm.recv() => {
                            tracing::info!("shutdown signal received (SIGTERM)");
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "failed to install SIGTERM handler; SIGINT only");
                    let _ = tokio::signal::ctrl_c().await;
                    tracing::info!("shutdown signal received (SIGINT)");
                }
            }
        }
        #[cfg(not(unix))]
        {
            if tokio::signal::ctrl_c().await.is_ok() {
                tracing::info!("shutdown signal received");
            }
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
    notification_router.shutdown().await;
    if let Some(worker) = federation_worker {
        worker.shutdown().await;
    }
    serve_result?;
    obs_guard.shutdown();
    Ok(())
}
