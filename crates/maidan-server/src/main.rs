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

/// What this process was asked to be.
#[derive(Debug, PartialEq, Eq)]
enum Invocation {
    /// Become the server (the only invocation that takes no arguments).
    Serve,
    /// Probe an already-running server and exit.
    HealthCheck,
    /// Something else, which is refused rather than ignored.
    Unknown(String),
}

fn classify_args(args: impl Iterator<Item = String>) -> Invocation {
    let mut invocation = Invocation::Serve;
    for arg in args {
        match arg.as_str() {
            "--health-check" => invocation = Invocation::HealthCheck,
            other => return Invocation::Unknown(other.to_string()),
        }
    }
    invocation
}

/// Probe this server's own readiness and exit 0 or 1.
///
/// The runtime image is `gcr.io/distroless/cc-debian12`, which has no shell and
/// no `curl` — so a compose or Kubernetes health check has nothing to call
/// *except* this binary. Running it as its own probe keeps the health check
/// honest (it speaks real HTTP to the real port) without adding a shell to a
/// distroless image just to be able to ask.
///
/// `/health/ready` is the deep check: degraded dependencies read as unhealthy,
/// which is what a `depends_on: service_healthy` wants to wait for.
async fn run_health_check() -> anyhow::Result<()> {
    // Read the port straight from `MAIDAN_BIND` rather than loading `Config`: a
    // probe that needs `DATABASE_URL` to decide whether the server is up has
    // acquired a way to fail that has nothing to do with the server.
    let port = std::env::var("MAIDAN_BIND")
        .ok()
        .and_then(|bind| bind.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()))
        .unwrap_or(8080);
    let url = format!("http://127.0.0.1:{port}/health/ready");
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .context("build health-check client")?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if response.status().is_success() {
        return Ok(());
    }
    anyhow::bail!("{url} returned {}", response.status())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Before anything is loaded: this invocation talks to an already-running
    // server rather than becoming one.
    //
    // An unrecognised argument is refused rather than ignored. Ignoring it means
    // a typo — or a flag this build predates — silently *starts a server*, which
    // is how a container health check running `maidan-server --health-check`
    // against an older binary bound the port a second time and reported the
    // container unhealthy forever.
    match classify_args(std::env::args().skip(1)) {
        Invocation::HealthCheck => return run_health_check().await,
        Invocation::Unknown(arg) => anyhow::bail!(
            "unrecognised argument `{arg}`. maidan-server takes no arguments \
             except --health-check; it is configured through the environment."
        ),
        Invocation::Serve => {}
    }

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
            let session_settings = config.db.postgres_session_settings();
            // Build a pool options with the same connection setup for the primary
            // and (when configured) the read replica. Caps every pooled
            // connection's queries; boot migrations exempt their own connection
            // (they reset statement_timeout under the advisory lock), and large
            // reindexes should use the CLI's own pool.
            let make_pg_opts = || {
                let mut o = PgPoolOptions::new()
                    .max_connections(max_connections)
                    .acquire_timeout(std::time::Duration::from_secs(acquire_timeout_secs));
                let settings = session_settings.clone();
                if !settings.is_empty() {
                    o = o.after_connect(move |conn, _meta| {
                        let settings = settings.clone();
                        Box::pin(async move {
                            for setting in &settings {
                                sqlx::query(setting).execute(&mut *conn).await?;
                            }
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
            // Read-replica pool: when MAIDAN_DB_REPLICA_URL is set, connect a
            // separate reader pool (validating replica reachability at boot) so
            // LSN-token read routing can send eligible reads there. Unset →
            // reads stay on the primary (unchanged).
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
            // Search gets its own replica reader pool so search reads honor the
            // same per-request consistency token as store reads. A separate
            // pool from the store's reader (pools are not shared across the
            // Arc).
            let pg_search = match config.replica_url.as_deref() {
                Some(replica_url) => {
                    let search_reader = make_pg_opts().connect(replica_url).await.context(
                        "connect to postgres read replica for search (MAIDAN_DB_REPLICA_URL)",
                    )?;
                    PostgresSearch::with_replica_reader(pool, search_reader)
                }
                None => PostgresSearch::new(pool),
            };
            // Capture the search routing counters for
            // `maidan_search_replica_reads_total` when a replica is configured;
            // `None` otherwise.
            search_read_routing_metrics = config
                .replica_url
                .is_some()
                .then(|| pg_search.read_routing_metrics());
            search = Arc::new(pg_search);
            use_embedding_indexer = true;
        }
        Dialect::Sqlite => {
            // Per-connection PRAGMAs (foreign_keys, busy_timeout, WAL) are
            // applied in the pool's after_connect so every pooled connection
            // enforces them — not just the first.
            let pool = maidan_search::sqlite_pool_options_with(config.db.busy_timeout_ms)
                // Serialize through one connection by default: a
                // multi-connection SQLite pool deadlocks concurrent
                // read-modify-write transactions. Overridable via
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
    // `hash-v1` is a deterministic hash, not a real embedding — semantic search
    // over it returns near-random results. Warn loudly at boot so a stranger
    // who leaves the provider unset isn't silently served meaningless
    // "semantic" hits.
    if embedding_provider.model_name() == "hash-v1" {
        tracing::warn!(
            "embedding provider is `hash-v1`: a deterministic hash, NOT semantically \
             meaningful — semantic search will return near-random results. Set \
             MAIDAN_EMBEDDING_PROVIDER=openai-compatible (+ MAIDAN_EMBEDDING_* config) for \
             real semantic search; lexical/full-text search is unaffected."
        );
    }

    // Register the active model in the per-model table scheme at boot so it's
    // queryable before the first write and a dimension mismatch surfaces now.
    // Non-fatal: embeddings are best-effort and the per-message path
    // re-attempts; the server must still serve messaging.
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
        if let Some(repair) = maidan_server::embed_repair::config_from_env() {
            let (search, provider, metrics) =
                (search.clone(), embedding_provider.clone(), metrics.clone());
            tokio::spawn(async move {
                maidan_server::embed_repair::run(search, provider, metrics, repair).await;
            });
        }
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
    // Key rotation: old keys from FEDERATION_DECRYPT_KEYS stay available for
    // decrypt after the primary is rotated. A malformed entry is a hard error —
    // silently dropping an old key would strand its ciphertexts.
    let decrypt_fallback_keys =
        maidan_auth::decrypt_fallback_keys_from_env().context("FEDERATION_DECRYPT_KEYS")?;
    if !decrypt_fallback_keys.is_empty() {
        tracing::info!(
            count = decrypt_fallback_keys.len(),
            "secret key rotation: decrypt fallback keys loaded"
        );
    }
    maidan_auth::init_decrypt_fallback_keys(decrypt_fallback_keys);

    let export_signing =
        maidan_auth::export_signing_key_from_env().context("MAIDAN_EXPORT_SIGNING_KEY")?;
    let export_verify_keys =
        maidan_auth::export_verify_keys_from_env().context("MAIDAN_EXPORT_VERIFY_KEYS")?;
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
    // MCP resource-update notifications fan out across replicas.
    state.attach_resource_notifier(resource_notifier);
    state.mcp.spawn_resource_notify_listener();
    // Presence/typing/roster fan out across replicas (Postgres NOTIFY).
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
    // backoff promptly. Only wired when the relay runs.
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
    // Default-on global rate limit: a deployment that configures nothing still
    // gets a DoS floor. `MAIDAN_RATE_LIMIT_MAX` (incl. `0`) overrides.
    state.rate_limit_default_on = true;
    // A2A Agent Card transport advertisement: public origin for absolute
    // interface URLs + the advertised gRPC address (§5.2 negotiation).
    state.a2a_card = maidan_server::a2a_agent::A2aCardConfig::from_env();
    // Read-replica routing: when a replica is configured, stamp the consistency
    // token on writes and route replica-eligible reads.
    state.read_replica_enabled = config.replica_url.is_some();
    state.read_routing_metrics = read_routing_metrics;
    state.search_read_routing_metrics = search_read_routing_metrics;

    // Experimental J-01 spike. Default-off and advisory-only: this separate
    // route never writes the land-gate pointer or enters the close path.
    if let Some(advisor) = maidan_server::land_gate_advisor::from_env()
        .await
        .context("configure Jev land-gate advisor")?
    {
        state.attach_land_gate_advisor(advisor);
        tracing::info!("experimental Jev land-gate advisor enabled");
    }

    if let Some(key) = export_signing {
        tracing::info!(
            public_key = %key.public_key_hex(),
            "workspace export signing configured"
        );
        state.attach_export_signing(key);
    } else {
        tracing::warn!(
            "MAIDAN_EXPORT_SIGNING_KEY unset; GET /workspaces/:id/export refuses until configured"
        );
    }
    if !export_verify_keys.is_empty() {
        tracing::info!(
            count = export_verify_keys.len(),
            "workspace export verify keyring loaded"
        );
        state.attach_export_verify_keys(export_verify_keys);
    }

    // Email transport: wire it only when `MAIDAN_SMTP_*` is configured —
    // otherwise the notification router sends no email.
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

    // Web Push sender: built from VAPID_* when configured. The router delivers
    // to a member's subscriptions when they have no live WS.
    if let Some(config) = maidan_server::web_push::WebPushConfig::from_env() {
        state.attach_web_push(std::sync::Arc::new(
            maidan_server::web_push::VapidWebPushSender::new(config),
        ));
        tracing::info!("web push (VAPID) configured");
    }

    // Background mail-outbox worker: drains the durable mail queue with
    // retry/backoff + dead-lettering. Runs whenever a transport is configured
    // (the router enqueues only then). Tick via `MAIDAN_MAIL_WORKER_TICK_SECS`
    // (default 5s).
    if state.mail.is_some() {
        let mail_state = state.clone();
        let mail_cfg = maidan_server::mail_worker::config_from_env();
        tokio::spawn(async move {
            maidan_server::mail_worker::run(mail_state, mail_cfg).await;
        });
    }

    // Slack projector: wire the ingress config when
    // `MAIDAN_SLACK_SIGNING_SECRET` is set — otherwise
    // `/integrations/slack/events` stays disabled (404). A projector, not a
    // bot: no LLM in Maidan.
    if let Some(slack_cfg) = maidan_server::slack::SlackConfig::from_env() {
        // Egress needs a bot token for chat.postMessage; ingress works without
        // one. The egress runs on the notification-router bus consumer.
        if let Some(bot_token) = slack_cfg.bot_token.clone() {
            state.attach_slack_sender(std::sync::Arc::new(
                maidan_server::slack::SlackWebClient::new(bot_token),
            ));
            tracing::info!("slack projector egress configured");
        }
        state.attach_slack(std::sync::Arc::new(slack_cfg));
        tracing::info!("slack projector ingress configured");
    }

    // Git/GitHub projector: wire the ingress config when
    // `MAIDAN_GITHUB_WEBHOOK_SECRET` is set — otherwise
    // `/integrations/github/events` stays disabled (404). A projector, not a
    // bot.
    if let Some(github_cfg) = maidan_server::github::GithubConfig::from_env() {
        // Egress needs a token for issue-comment posts; ingress works without
        // one. The egress runs on the notification-router bus consumer.
        if let Some(token) = github_cfg.api_token.clone() {
            state.attach_github_sender(std::sync::Arc::new(
                maidan_server::github::GithubApiClient::new(token),
            ));
            tracing::info!("github projector egress configured");
        }
        state.attach_github(std::sync::Arc::new(github_cfg));
        tracing::info!("github projector ingress configured");
    }

    // Background projector-egress worker: drains the durable egress queue with
    // retry/backoff + dead-lettering. Runs whenever a projector sender is
    // configured — which is exactly when the projectors enqueue, so a
    // deployment without one neither queues nor drains. Tick via
    // `MAIDAN_EGRESS_WORKER_TICK_SECS` (default 5s).
    if state.slack_sender.is_some() || state.github_sender.is_some() {
        let egress_state = state.clone();
        let egress_cfg = maidan_server::egress_worker::config_from_env();
        tokio::spawn(async move {
            maidan_server::egress_worker::run(egress_state, egress_cfg).await;
        });
    }

    // Give the MCP server the slash-command dispatcher so an MCP `post_message`
    // runs registered slash commands like a REST post. Attached last (after
    // every other `attach_*`) so the dispatcher's `AppState` clone is fully
    // configured; server-binary only, so tests/embedders skip slash dispatch.
    state.mcp.set_slash_dispatcher(std::sync::Arc::new(
        maidan_server::slash_commands::ServerSlashDispatcher::new(state.clone()),
    ));
    // The at-rest encryption key powers `resolve_secret` over MCP, mirroring
    // the REST resolve; unset means the tool reports it's unavailable.
    if let Some(key) = state.federation.encryption_key.clone() {
        state.mcp.set_encryption_key(key);
    }
    // MCP export/verify/import share the REST operator keyring.
    if let Some(key) = state.export_signing.clone() {
        state.mcp.set_export_signing(key);
    }
    if !state.export_verify_keys.is_empty() {
        state
            .mcp
            .set_export_verify_keys(state.export_verify_keys.clone());
    }

    // Background data-retention sweeper: opt-in via `MAIDAN_RETENTION_*_DAYS`.
    // Prunes the event log (floored at the durable delivery watermark), audit
    // trail, and delivery tables past their age.
    if let Some(retention_cfg) = maidan_server::retention::config_from_env() {
        let retention_store = state.store.clone();
        tokio::spawn(async move {
            maidan_server::retention::run(retention_store, retention_cfg).await;
        });
    }

    // The scheduled half of the decision. The tap verifies what it projects;
    // this verifies the chain. Opt-in — unset means an unconfigured deployment
    // is unchanged.
    if let Some(interval) = maidan_server::chain_verify::interval_from_env() {
        let verify_store = state.store.clone();
        tokio::spawn(async move {
            maidan_server::chain_verify::run(verify_store, interval).await;
        });
    }

    // Background scheduled/recurring-task sweeper: opt-in via
    // `MAIDAN_SCHEDULER_TICK_SECS`. Materializes a task thread for each
    // schedule that comes due.
    if let Some(scheduler_cfg) = maidan_server::scheduler::config_from_env() {
        let scheduler_state = state.clone();
        tokio::spawn(async move {
            maidan_server::scheduler::run(scheduler_state, scheduler_cfg).await;
        });
    }

    // Background wait-timer sweeper: opt-in via `MAIDAN_WAIT_SWEEP_TICK_SECS`.
    // Fires each thread wait past its deadline — emitting `WaitTimedOut` and
    // (per policy) parking the thread.
    if let Some(wait_cfg) = maidan_server::wait_sweeper::config_from_env() {
        let wait_state = state.clone();
        tokio::spawn(async move {
            maidan_server::wait_sweeper::run(wait_state, wait_cfg).await;
        });
    }

    // Background email-digest sweeper: opt-in via `MAIDAN_DIGEST_TICK_SECS`.
    // Emails digest-mode members an unread rollup.
    if let Some(digest_cfg) = maidan_server::digest::config_from_env() {
        let digest_state = state.clone();
        tokio::spawn(async move {
            maidan_server::digest::run(digest_state, digest_cfg).await;
        });
    }

    // A2A gRPC binding: opt-in via `MAIDAN_A2A_GRPC_ADDR` (e.g.
    // `0.0.0.0:50051`). Serves the tonic A2AService on a separate port; the
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

    let indexer = Indexer::new(bus, indexer_handler)
        .with_log(state.store.clone())
        .spawn_with_heartbeat(indexer_heartbeat);
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
    //
    // Kubernetes removes a terminating pod from its Service endpoints at the
    // same time as it sends SIGTERM, so for a moment new requests still arrive.
    // Readiness fails first and the listener stays open for the drain delay;
    // the runtime image has no shell, so this cannot be an exec `preStop`.
    let draining = state.draining.clone();
    let drain = maidan_server::shutdown_drain_from_env();
    let shutdown = async move {
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
        draining.store(true, std::sync::atomic::Ordering::Relaxed);
        if !drain.is_zero() {
            tracing::info!(
                secs = drain.as_secs(),
                "draining before closing the listener"
            );
            tokio::time::sleep(drain).await;
        }
    };

    let serve_result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
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

#[cfg(test)]
mod invocation_tests {
    use super::*;

    fn classify(args: &[&str]) -> Invocation {
        classify_args(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn no_arguments_means_serve() {
        assert_eq!(classify(&[]), Invocation::Serve);
    }

    #[test]
    fn the_health_check_flag_is_recognised() {
        assert_eq!(classify(&["--health-check"]), Invocation::HealthCheck);
    }

    /// The case that cost two minutes per container start: a released binary
    /// that predates `--health-check` ignored it, started a second server, and
    /// failed on the bound port — so the health check could never pass and the
    /// container was unhealthy forever.
    #[test]
    fn an_unrecognised_argument_is_refused_rather_than_ignored() {
        assert_eq!(
            classify(&["--helth-check"]),
            Invocation::Unknown("--helth-check".to_string())
        );
        assert_eq!(
            classify(&["serve"]),
            Invocation::Unknown("serve".to_string())
        );
    }
}
