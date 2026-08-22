//! Configuration loaded from environment variables.
//!
//! No file loader yet — `.env`-style files are picked up by whatever
//! launches the process (compose, k8s, direnv). Keeping config pure-env
//! at this stage avoids a precedence puzzle we don't need.

use std::{net::SocketAddr, path::PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required env var: {0}")]
    Missing(&'static str),

    #[error("invalid value for {0}: {1}")]
    Invalid(&'static str, String),
}

#[derive(Debug, Clone)]
pub enum ArtifactBackend {
    LocalFs {
        root: PathBuf,
    },
    S3 {
        endpoint: String,
        bucket: String,
        region: String,
        access_key: String,
        secret_key: String,
    },
}

/// Database connection-pool and timeout tuning (Cluster 107). All fields are
/// env-driven with defaults that reproduce the previous hardcoded behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbConfig {
    /// Pool max connections. `None` keeps the dialect default (Postgres 16,
    /// SQLite 8) so the default reproduces prior behavior.
    pub max_connections: Option<u32>,
    /// Seconds to wait for a free pooled connection before erroring
    /// (matches sqlx's prior 30 s default).
    pub acquire_timeout_secs: u64,
    /// Postgres per-connection `statement_timeout` in ms; `0` disables it.
    /// Defaults to 30 000 (30 s) so a runaway query can't pin a pooled
    /// connection indefinitely; boot migrations exempt themselves. Set
    /// `MAIDAN_DB_STATEMENT_TIMEOUT_MS=0` to restore the uncapped behavior.
    pub statement_timeout_ms: u64,
    /// SQLite `busy_timeout` in ms (default 5000, as before).
    pub busy_timeout_ms: u64,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            max_connections: None,
            acquire_timeout_secs: 30,
            statement_timeout_ms: 30_000,
            busy_timeout_ms: 5000,
        }
    }
}

impl DbConfig {
    /// Parse from a generic lookup so the logic is unit-testable without
    /// touching the process environment.
    fn from_lookup(get: impl Fn(&'static str) -> Option<String>) -> Result<Self, ConfigError> {
        fn num<T: std::str::FromStr>(
            name: &'static str,
            get: &impl Fn(&'static str) -> Option<String>,
        ) -> Result<Option<T>, ConfigError> {
            match get(name) {
                Some(v) => v
                    .trim()
                    .parse::<T>()
                    .map(Some)
                    .map_err(|_| ConfigError::Invalid(name, v)),
                None => Ok(None),
            }
        }
        let default = DbConfig::default();
        Ok(Self {
            max_connections: num::<u32>("MAIDAN_DB_MAX_CONNECTIONS", &get)?,
            acquire_timeout_secs: num::<u64>("MAIDAN_DB_ACQUIRE_TIMEOUT_SECS", &get)?
                .unwrap_or(default.acquire_timeout_secs),
            statement_timeout_ms: num::<u64>("MAIDAN_DB_STATEMENT_TIMEOUT_MS", &get)?
                .unwrap_or(default.statement_timeout_ms),
            busy_timeout_ms: num::<u64>("MAIDAN_DB_BUSY_TIMEOUT_MS", &get)?
                .unwrap_or(default.busy_timeout_ms),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub database_url: String,
    /// Optional read-replica connection string (`MAIDAN_DB_REPLICA_URL`). When set,
    /// the store gets a separate reader pool for LSN-token read routing (Cluster
    /// 262+); unset means reads stay on the primary (unchanged).
    pub replica_url: Option<String>,
    pub artifact_backend: ArtifactBackend,
    pub log_filter: String,
    pub db: DbConfig,
}

pub fn is_production() -> bool {
    std::env::var("MAIDAN_ENV").as_deref() == Ok("production")
}

fn env_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

/// Whether the operator asked to run with authentication disabled.
pub fn auth_disabled_requested() -> bool {
    env_truthy("AUTH_DISABLED")
}

/// Whether the operator has *explicitly acknowledged* running without any
/// authentication. `AUTH_DISABLED` is fail-closed: it takes effect only when
/// this is also set (and never in production).
pub fn insecure_no_auth_acknowledged() -> bool {
    env_truthy("MAIDAN_ALLOW_INSECURE_NO_AUTH")
}

/// Fail-closed gate for `AUTH_DISABLED`. Pure over its inputs so it is unit
/// tested without touching the process environment. Disabling auth is rejected
/// in production outright, and otherwise requires the explicit
/// `MAIDAN_ALLOW_INSECURE_NO_AUTH` acknowledgement — so a stray `AUTH_DISABLED=1`
/// (e.g. copied from a dev compose file) fails startup loudly instead of
/// silently serving an unauthenticated workspace.
pub(crate) fn validate_insecure_no_auth(
    requested: bool,
    production: bool,
    acknowledged: bool,
) -> Result<(), ConfigError> {
    if requested {
        if production {
            return Err(ConfigError::Invalid(
                "AUTH_DISABLED",
                "cannot be set when MAIDAN_ENV=production".into(),
            ));
        }
        if !acknowledged {
            return Err(ConfigError::Invalid(
                "AUTH_DISABLED",
                "running without authentication requires MAIDAN_ALLOW_INSECURE_NO_AUTH=1 to \
                 acknowledge the risk (never set this in production)"
                    .into(),
            ));
        }
    }
    Ok(())
}

impl Config {
    /// Load from `std::env`. Errors return [`ConfigError`] with the
    /// offending key.
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind: SocketAddr = std::env::var("MAIDAN_BIND")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
            .parse()
            .map_err(|e: std::net::AddrParseError| {
                ConfigError::Invalid("MAIDAN_BIND", e.to_string())
            })?;

        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| ConfigError::Missing("DATABASE_URL"))?;
        let replica_url = std::env::var("MAIDAN_DB_REPLICA_URL")
            .ok()
            .filter(|s| !s.trim().is_empty());

        let backend = std::env::var("ARTIFACT_BACKEND").unwrap_or_else(|_| "localfs".to_string());
        let artifact_backend = match backend.as_str() {
            "localfs" => {
                let root: PathBuf = std::env::var("ARTIFACT_LOCALFS_ROOT")
                    .unwrap_or_else(|_| "./.local/artifacts".to_string())
                    .into();
                ArtifactBackend::LocalFs { root }
            }
            "s3" => {
                fn require(name: &'static str) -> Result<String, ConfigError> {
                    std::env::var(name).map_err(|_| ConfigError::Missing(name))
                }
                ArtifactBackend::S3 {
                    endpoint: require("S3_ENDPOINT")?,
                    bucket: require("S3_BUCKET")?,
                    region: std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
                    access_key: require("S3_ACCESS_KEY_ID")?,
                    secret_key: require("S3_SECRET_ACCESS_KEY")?,
                }
            }
            other => return Err(ConfigError::Invalid("ARTIFACT_BACKEND", other.to_string())),
        };

        let log_filter =
            std::env::var("MAIDAN_LOG").unwrap_or_else(|_| "info,sqlx=warn".to_string());

        validate_insecure_no_auth(
            auth_disabled_requested(),
            is_production(),
            insecure_no_auth_acknowledged(),
        )?;

        let db = DbConfig::from_lookup(|name| std::env::var(name).ok())?;

        Ok(Self {
            bind,
            database_url,
            replica_url,
            artifact_backend,
            log_filter,
            db,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&'static str, &str)]) -> impl Fn(&'static str) -> Option<String> {
        let map: HashMap<&'static str, String> =
            pairs.iter().map(|(k, v)| (*k, v.to_string())).collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn auth_disabled_is_fail_closed() {
        // Not requested → always fine regardless of the other flags.
        assert!(validate_insecure_no_auth(false, false, false).is_ok());
        assert!(validate_insecure_no_auth(false, true, false).is_ok());
        // Requested + acknowledged (non-prod) → allowed.
        assert!(validate_insecure_no_auth(true, false, true).is_ok());
        // Requested without the explicit acknowledgement → rejected.
        assert!(matches!(
            validate_insecure_no_auth(true, false, false),
            Err(ConfigError::Invalid("AUTH_DISABLED", _))
        ));
        // Requested in production → rejected even with the acknowledgement.
        assert!(matches!(
            validate_insecure_no_auth(true, true, true),
            Err(ConfigError::Invalid("AUTH_DISABLED", _))
        ));
    }

    #[test]
    fn db_config_defaults_are_safe() {
        let cfg = DbConfig::from_lookup(lookup(&[])).unwrap();
        assert_eq!(cfg, DbConfig::default());
        assert_eq!(cfg.max_connections, None); // dialect default (pg 16 / sqlite 8)
        assert_eq!(cfg.acquire_timeout_secs, 30);
        assert_eq!(cfg.statement_timeout_ms, 30_000); // 30 s cap by default
        assert_eq!(cfg.busy_timeout_ms, 5000);
    }

    #[test]
    fn db_config_statement_timeout_can_be_disabled() {
        let cfg =
            DbConfig::from_lookup(lookup(&[("MAIDAN_DB_STATEMENT_TIMEOUT_MS", "0")])).unwrap();
        assert_eq!(cfg.statement_timeout_ms, 0); // 0 restores the uncapped behavior
    }

    #[test]
    fn db_config_reads_env_overrides() {
        let cfg = DbConfig::from_lookup(lookup(&[
            ("MAIDAN_DB_MAX_CONNECTIONS", "32"),
            ("MAIDAN_DB_ACQUIRE_TIMEOUT_SECS", "5"),
            ("MAIDAN_DB_STATEMENT_TIMEOUT_MS", "15000"),
            ("MAIDAN_DB_BUSY_TIMEOUT_MS", "10000"),
        ]))
        .unwrap();
        assert_eq!(cfg.max_connections, Some(32));
        assert_eq!(cfg.acquire_timeout_secs, 5);
        assert_eq!(cfg.statement_timeout_ms, 15000);
        assert_eq!(cfg.busy_timeout_ms, 10000);
    }

    #[test]
    fn db_config_rejects_non_numeric() {
        let err =
            DbConfig::from_lookup(lookup(&[("MAIDAN_DB_MAX_CONNECTIONS", "lots")])).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid("MAIDAN_DB_MAX_CONNECTIONS", _)
        ));
    }
}
