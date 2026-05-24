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

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub database_url: String,
    pub artifact_backend: ArtifactBackend,
    pub log_filter: String,
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

        if std::env::var("MAIDAN_ENV").as_deref() == Ok("production")
            && matches!(
                std::env::var("AUTH_DISABLED").as_deref(),
                Ok("1") | Ok("true") | Ok("TRUE")
            )
        {
            return Err(ConfigError::Invalid(
                "AUTH_DISABLED",
                "cannot be set when MAIDAN_ENV=production".into(),
            ));
        }

        Ok(Self {
            bind,
            database_url,
            artifact_backend,
            log_filter,
        })
    }
}
