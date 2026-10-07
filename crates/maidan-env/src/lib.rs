//! Every `MAIDAN_*` environment variable the server knows (F-52).
//!
//! Configuration is read where it is used, across a dozen crates, so a
//! misspelt variable (`MAIDAN_RATE_LIMT_MAX`) configures nothing and says
//! nothing: the server boots on the default the operator meant to override.
//! Listing every name here lets the server and the `maidan` CLI refuse any
//! other `MAIDAN_*` variable, with the nearest known name as a hint, without
//! pulling the reads themselves into one struct. `env_registry_contract` keeps
//! the list honest: every name the server's crates read is on it, and nothing
//! is on it that nothing reads.

mod secret_files;

pub use secret_files::{load_secret_files, resolve_secret_files, SecretFileError, SECRET_FILE_ENV};

/// Set to `1` to start with unknown `MAIDAN_*` variables, logging them instead.
pub const ALLOW_UNKNOWN_ENV: &str = "MAIDAN_ALLOW_UNKNOWN_ENV";

/// Variables the server process reads.
pub const SERVER_ENV: &[&str] = &[
    "MAIDAN_A2A_GRPC_ADDR",
    "MAIDAN_A2A_GRPC_PLAINTEXT",
    "MAIDAN_A2A_GRPC_PUBLIC_ADDR",
    "MAIDAN_A2A_PUBLIC_ORIGIN",
    "MAIDAN_ALLOWED_HOSTS",
    "MAIDAN_ALLOW_INSECURE_DEV_KEK",
    "MAIDAN_ALLOW_INSECURE_NO_AUTH",
    "MAIDAN_ALLOW_PRIVATE_EGRESS",
    "MAIDAN_ALLOW_UNKNOWN_ENV",
    "MAIDAN_AUTOMATION_MAX_ATTEMPTS",
    "MAIDAN_AUTOMATION_POLL_INTERVAL_MS",
    "MAIDAN_BIND",
    "MAIDAN_BOOTSTRAP",
    "MAIDAN_BUS_BROADCAST_CAP",
    "MAIDAN_CHAIN_VERIFY_SECS",
    "MAIDAN_CLAIM_ACK_TIMEOUT_SECS",
    "MAIDAN_CLAIM_DEFAULT_LEASE_SECS",
    "MAIDAN_CLAIM_REAP_TICK_SECS",
    "MAIDAN_CONTENT_KEK",
    "MAIDAN_CONTENT_KEK_FILE",
    "MAIDAN_CONTENT_KEK_PREVIOUS",
    "MAIDAN_COOKIE_SECURE",
    "MAIDAN_DB_ACQUIRE_TIMEOUT_SECS",
    "MAIDAN_DB_BUSY_TIMEOUT_MS",
    "MAIDAN_DB_IDLE_IN_TRANSACTION_TIMEOUT_MS",
    "MAIDAN_DB_LOCK_TIMEOUT_MS",
    "MAIDAN_DB_MAX_CONNECTIONS",
    "MAIDAN_DB_REPLICA_URL",
    "MAIDAN_DB_STATEMENT_TIMEOUT_MS",
    "MAIDAN_DELIVERY_RECONCILE_MS",
    "MAIDAN_DELIVERY_STABILITY_SECS",
    "MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE",
    "MAIDAN_DIGEST_TICK_SECS",
    "MAIDAN_EGRESS_WORKER_TICK_SECS",
    "MAIDAN_EMAIL_PRESENCE_WINDOW_SECS",
    "MAIDAN_EMBEDDING_API_KEY",
    "MAIDAN_EMBEDDING_DIM",
    "MAIDAN_EMBEDDING_ENDPOINT",
    "MAIDAN_EMBEDDING_MODEL",
    "MAIDAN_EMBEDDING_PROVIDER",
    "MAIDAN_EMBEDDING_TIMEOUT_SECS",
    "MAIDAN_EMBED_REPAIR_BATCH",
    "MAIDAN_EMBED_REPAIR_INTERVAL_SECS",
    "MAIDAN_ENV",
    "MAIDAN_EXPORT_SIGNING_KEY",
    "MAIDAN_EXPORT_VERIFY_KEYS",
    "MAIDAN_GITHUB_TOKEN",
    "MAIDAN_GITHUB_TOKEN_FILE",
    "MAIDAN_GITHUB_WEBHOOK_SECRET",
    "MAIDAN_GITHUB_WEBHOOK_SECRET_FILE",
    "MAIDAN_GITHUB_WRITE_REPOS",
    "MAIDAN_HNSW_EF_CONSTRUCTION",
    "MAIDAN_HNSW_EF_SEARCH",
    "MAIDAN_HNSW_M",
    "MAIDAN_INDEXER_BATCH_SIZE",
    "MAIDAN_INDEXER_QUEUE_CAPACITY",
    "MAIDAN_INDEXER_RETRIES",
    "MAIDAN_INDEXER_RETRY_BASE_MS",
    "MAIDAN_JEV_BASE_URL",
    "MAIDAN_JEV_GREEN_MIN_CONFIDENCE",
    "MAIDAN_JEV_LAND_GATE_ENABLED",
    "MAIDAN_JEV_MODEL",
    "MAIDAN_JEV_RED_MIN_CONFIDENCE",
    "MAIDAN_JEV_TIMEOUT_MS",
    "MAIDAN_LOG",
    "MAIDAN_LOG_FORMAT",
    "MAIDAN_MAIL_WORKER_TICK_SECS",
    "MAIDAN_MARK_READY_APP_ID",
    "MAIDAN_MAX_BODY_BYTES",
    "MAIDAN_MAX_CONCURRENT_REQUESTS",
    "MAIDAN_MAX_WS_CONNECTIONS",
    "MAIDAN_MCP_STREAMABLE_SESSION_TTL_SECS",
    "MAIDAN_OIDC_AUTO_MINT",
    "MAIDAN_OIDC_AUTO_PROVISION",
    "MAIDAN_OIDC_CLIENT_ID",
    "MAIDAN_OIDC_CLIENT_SECRET",
    "MAIDAN_OIDC_ENABLED",
    "MAIDAN_OIDC_FIRST_ADMIN",
    "MAIDAN_OIDC_ISSUER",
    "MAIDAN_OIDC_LINK_EMAIL",
    "MAIDAN_OIDC_MOCK",
    "MAIDAN_OIDC_PENDING_TTL_SECS",
    "MAIDAN_OIDC_POST_LOGOUT_REDIRECT_URI",
    "MAIDAN_OIDC_REDIRECT_URI",
    "MAIDAN_OIDC_SCOPES",
    "MAIDAN_OUTBOX_MAX_ATTEMPTS",
    "MAIDAN_OUTBOX_MAX_POLL_INTERVAL_MS",
    "MAIDAN_OUTBOX_POLL_INTERVAL_MS",
    "MAIDAN_OUTBOX_RELAY",
    "MAIDAN_OUTBOX_RELAY_MODE",
    "MAIDAN_PRESENCE_HEARTBEAT_SECS",
    "MAIDAN_PRESENCE_TTL_SECS",
    "MAIDAN_RATE_LIMIT_MAX",
    "MAIDAN_RATE_LIMIT_REDIS_URL",
    "MAIDAN_RATE_LIMIT_WINDOW_SECS",
    "MAIDAN_RETENTION_AUDIT_DAYS",
    "MAIDAN_RETENTION_BATCH",
    "MAIDAN_RETENTION_DELIVERIES_DAYS",
    "MAIDAN_RETENTION_EVENTS_DAYS",
    "MAIDAN_RETENTION_MESSAGES_DAYS",
    "MAIDAN_RETENTION_NOTIFICATIONS_DAYS",
    "MAIDAN_RETENTION_SWEEP_SECS",
    "MAIDAN_SCHEDULER_TICK_SECS",
    "MAIDAN_SECRET_EGRESS_ALLOWLIST",
    "MAIDAN_SESSION_SECRET",
    "MAIDAN_SESSION_SECRET_FILE",
    "MAIDAN_SESSION_TTL_SECS",
    "MAIDAN_SHUTDOWN_DRAIN_SECS",
    "MAIDAN_SLACK_BOT_TOKEN",
    "MAIDAN_SLACK_BOT_TOKEN_FILE",
    "MAIDAN_SLACK_SIGNING_SECRET",
    "MAIDAN_SLACK_SIGNING_SECRET_FILE",
    "MAIDAN_SMTP_FROM",
    "MAIDAN_SMTP_HOST",
    "MAIDAN_SMTP_PASSWORD",
    "MAIDAN_SMTP_PORT",
    "MAIDAN_SMTP_STARTTLS",
    "MAIDAN_SMTP_USERNAME",
    "MAIDAN_SUBSCRIBE_RESUME_SECRET",
    "MAIDAN_SUBSCRIBE_RESUME_TTL_SECS",
    "MAIDAN_TRUSTED_PROXY_HOPS",
    "MAIDAN_VAPID_PRIVATE_KEY",
    "MAIDAN_VAPID_PUBLIC_KEY",
    "MAIDAN_VAPID_SUBJECT",
    "MAIDAN_WAIT_SWEEP_TICK_SECS",
    "MAIDAN_WEBHOOK_MAX_ATTEMPTS",
    "MAIDAN_WEBHOOK_POLL_INTERVAL_MS",
    "MAIDAN_WEBPUSH_LIVE_WINDOW_SECS",
    "MAIDAN_WEBPUSH_WORKER_TICK_SECS",
    "MAIDAN_WORKSPACE_RATE_LIMIT_MAX",
    "MAIDAN_WORKSPACE_RATE_LIMIT_WINDOW_SECS",
];

/// Variables the server does not read but a shell that starts it may carry for
/// something else: the CLI and the SDKs, scripts and compose interpolation,
/// test harnesses, and image build arguments.
pub const TOLERATED_ENV: &[&str] = &[
    "MAIDAN_A2A_GRPC_PORT",
    "MAIDAN_A2A_PORT",
    "MAIDAN_A_PEER_URL",
    "MAIDAN_A_URL",
    "MAIDAN_BIN_DIR",
    "MAIDAN_B_URL",
    "MAIDAN_CHANNEL",
    "MAIDAN_CHAOS_DELAY_MS",
    "MAIDAN_CHAOS_KILL_EVERY",
    "MAIDAN_CHAOS_OPS",
    "MAIDAN_CONFORMANCE_OUT",
    "MAIDAN_CONFORMANCE_VERSION",
    "MAIDAN_DEMO_PORT",
    "MAIDAN_ENABLE_BOOTSTRAP",
    "MAIDAN_HOST_PORT",
    "MAIDAN_IMAGE",
    "MAIDAN_INSPECTOR_VERSION",
    "MAIDAN_LOADGEN_BEARER",
    "MAIDAN_LOADGEN_CONCURRENCY",
    "MAIDAN_LOADGEN_DURATION_SECS",
    "MAIDAN_LOADGEN_IDS",
    "MAIDAN_LOADGEN_OBSERVER_OPS",
    "MAIDAN_LOADGEN_OPS",
    "MAIDAN_LOADGEN_URL",
    "MAIDAN_MCP_PORT",
    "MAIDAN_MCP_TOKEN",
    "MAIDAN_NETWORK",
    "MAIDAN_OPENAPI_PORT",
    "MAIDAN_OTLP_PORT",
    "MAIDAN_OUTPUT_DIR",
    "MAIDAN_PEER_HOST_PORT",
    "MAIDAN_PRIMARY_PORT",
    "MAIDAN_PRIMARY_URL",
    "MAIDAN_REDOCLY_VERSION",
    "MAIDAN_REPLICA_PORT",
    "MAIDAN_REPLICA_URL",
    "MAIDAN_SCALE_LB_PORT",
    "MAIDAN_SDK_PORT",
    "MAIDAN_SHA256_AMD64",
    "MAIDAN_SHA256_ARM64",
    "MAIDAN_SIM_SEED",
    "MAIDAN_SIM_SEEDS",
    "MAIDAN_TAG",
    "MAIDAN_TASK",
    "MAIDAN_THREAD_ID",
    "MAIDAN_TOKEN",
    "MAIDAN_URL",
    "MAIDAN_VERSION",
    "MAIDAN_WORKSPACE",
    "MAIDAN_WORKSPACE_NAME",
    "MAIDAN_WRITE_LEXICON",
    "MAIDAN_WRITE_PORTABLE_GOLDENS",
];

/// A `MAIDAN_*` variable the server does not know, with the known name it is
/// closest to when one is close enough to be the likely intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownVar {
    pub name: String,
    pub suggestion: Option<&'static str>,
}

/// The `MAIDAN_*` names among `names` that are on neither list. `in_kubernetes`
/// tolerates the service links Kubernetes injects for every Service in the
/// namespace: a Service named `maidan-…` becomes `MAIDAN_…_SERVICE_HOST`,
/// `MAIDAN_…_PORT_8080_TCP` and the rest in every pod.
pub fn unknown_vars<I>(names: I, in_kubernetes: bool) -> Vec<UnknownVar>
where
    I: IntoIterator<Item = String>,
{
    let mut unknown: Vec<UnknownVar> = names
        .into_iter()
        .filter(|name| name.starts_with("MAIDAN_"))
        .filter(|name| !is_known(name))
        .filter(|name| !(in_kubernetes && is_service_link(name)))
        .map(|name| {
            let suggestion = suggest(&name);
            UnknownVar { name, suggestion }
        })
        .collect();
    unknown.sort_by(|a, b| a.name.cmp(&b.name));
    unknown.dedup_by(|a, b| a.name == b.name);
    unknown
}

/// The names Kubernetes generates for a Service `S` with a port named `N`:
/// `S_SERVICE_HOST`, `S_SERVICE_PORT`, `S_SERVICE_PORT_N`, `S_PORT`,
/// `S_PORT_<port>_<proto>` and that followed by `_ADDR`, `_PORT` or `_PROTO`.
fn is_service_link(name: &str) -> bool {
    if name.ends_with("_SERVICE_HOST") || name.ends_with("_SERVICE_PORT") {
        return true;
    }
    if name.contains("_SERVICE_PORT_") {
        return true;
    }
    let parts: Vec<&str> = name.split('_').collect();
    let port_at = |i: usize| {
        parts.get(i) == Some(&"PORT")
            && parts.get(i + 1).is_some_and(|p| p.parse::<u16>().is_ok())
            && parts
                .get(i + 2)
                .is_some_and(|p| matches!(*p, "TCP" | "UDP" | "SCTP"))
    };
    match parts.len() {
        n if n >= 2 && parts[n - 1] == "PORT" => true,
        n if n >= 4 && port_at(n - 3) => true,
        n if n >= 5 && matches!(parts[n - 1], "ADDR" | "PORT" | "PROTO") && port_at(n - 4) => true,
        _ => false,
    }
}

fn is_known(name: &str) -> bool {
    SERVER_ENV.contains(&name) || TOLERATED_ENV.contains(&name)
}

/// The server variable nearest `name`, if it is a plausible typo of one. Only
/// server names are offered: a tolerated name is never what an operator
/// configuring the server meant.
fn suggest(name: &str) -> Option<&'static str> {
    let limit = (name.len() / 6).clamp(1, 3);
    SERVER_ENV
        .iter()
        .map(|known| (edit_distance(name, known), *known))
        .filter(|(distance, _)| *distance <= limit)
        .min()
        .map(|(_, known)| known)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut current = Vec::with_capacity(b.len() + 1);
        current.push(i + 1);
        for (j, cb) in b.iter().enumerate() {
            let substitute = previous[j] + usize::from(ca != *cb);
            let insert = current[j] + 1;
            let delete = previous[j + 1] + 1;
            current.push(substitute.min(insert).min(delete));
        }
        previous = current;
    }
    previous[b.len()]
}

/// One line naming each unknown variable and its likely intent.
pub fn describe(unknown: &[UnknownVar]) -> String {
    unknown
        .iter()
        .map(|var| match var.suggestion {
            Some(known) => format!("{} (did you mean {known}?)", var.name),
            None => var.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn a_misspelt_server_variable_is_unknown_and_names_what_was_meant() {
        let unknown = unknown_vars(names(&["MAIDAN_RATE_LIMT_MAX", "MAIDAN_BIND"]), false);
        assert_eq!(
            unknown,
            vec![UnknownVar {
                name: "MAIDAN_RATE_LIMT_MAX".into(),
                suggestion: Some("MAIDAN_RATE_LIMIT_MAX"),
            }]
        );
        assert_eq!(
            describe(&unknown),
            "MAIDAN_RATE_LIMT_MAX (did you mean MAIDAN_RATE_LIMIT_MAX?)"
        );
    }

    #[test]
    fn known_tolerated_and_foreign_variables_pass() {
        let unknown = unknown_vars(
            names(&["MAIDAN_CONTENT_KEK", "MAIDAN_TOKEN", "DATABASE_URL", "PATH"]),
            false,
        );
        assert!(unknown.is_empty(), "{unknown:?}");
    }

    #[test]
    fn a_name_far_from_every_server_variable_gets_no_suggestion() {
        let unknown = unknown_vars(names(&["MAIDAN_FROBNICATE_EVERYTHING"]), false);
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0].suggestion, None);
    }

    #[test]
    fn kubernetes_service_links_pass_in_a_pod_and_only_there() {
        // What a Service `maidan-maidan` with a port named `http` injects.
        let links = names(&[
            "MAIDAN_MAIDAN_SERVICE_HOST",
            "MAIDAN_MAIDAN_SERVICE_PORT",
            "MAIDAN_MAIDAN_SERVICE_PORT_HTTP",
            "MAIDAN_MAIDAN_PORT",
            "MAIDAN_MAIDAN_PORT_8080_TCP",
            "MAIDAN_MAIDAN_PORT_8080_TCP_ADDR",
            "MAIDAN_MAIDAN_PORT_8080_TCP_PORT",
            "MAIDAN_MAIDAN_PORT_8080_TCP_PROTO",
        ]);
        assert!(unknown_vars(links.clone(), true).is_empty());
        assert_eq!(unknown_vars(links, false).len(), 8);
        let typo = unknown_vars(names(&["MAIDAN_RATE_LIMT_MAX"]), true);
        assert_eq!(typo.len(), 1, "a typo is still a typo in a pod");
    }

    #[test]
    fn the_lists_are_sorted_unique_and_disjoint() {
        for list in [SERVER_ENV, TOLERATED_ENV] {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted, list, "keep the list sorted and unique");
        }
        for name in TOLERATED_ENV {
            assert!(!SERVER_ENV.contains(name), "{name} is on both lists");
        }
        assert!(SERVER_ENV.contains(&ALLOW_UNKNOWN_ENV));
    }

    #[test]
    fn every_maidan_secret_file_variable_passes_the_boot_check() {
        let file_vars: Vec<String> = SECRET_FILE_ENV
            .iter()
            .map(|(_, file_var)| file_var.to_string())
            .collect();
        assert!(file_vars.iter().any(|v| v.starts_with("MAIDAN_")));
        let unknown = unknown_vars(file_vars, false);
        assert!(unknown.is_empty(), "{unknown:?}");
    }
}
