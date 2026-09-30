//! Configuration for subscribe resume tokens (WS + MCP SSE). The token codec
//! is [`maidan_auth::subscribe`].

use std::sync::Arc;

use crate::config::ConfigError;
use crate::session::session_secret_from_env;

/// Fixed secret for integration tests (`AppState::for_tests`).
pub const TEST_SUBSCRIBE_RESUME_SECRET: &[u8] = b"test-subscribe-resume-secret-32b!!";

pub fn secret_from_env() -> Result<Arc<[u8]>, ConfigError> {
    if let Ok(raw) = std::env::var("MAIDAN_SUBSCRIBE_RESUME_SECRET") {
        return validate_secret(&raw);
    }
    session_secret_from_env()
}

pub fn ttl_secs_from_env() -> u64 {
    std::env::var("MAIDAN_SUBSCRIBE_RESUME_TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(3600)
}

fn validate_secret(raw: &str) -> Result<Arc<[u8]>, ConfigError> {
    if raw.len() < 32 {
        return Err(ConfigError::Invalid(
            "MAIDAN_SUBSCRIBE_RESUME_SECRET",
            "must be at least 32 bytes".into(),
        ));
    }
    Ok(Arc::from(raw.as_bytes()))
}
