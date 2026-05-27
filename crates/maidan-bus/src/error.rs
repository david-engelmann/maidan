use thiserror::Error;

#[derive(Debug, Error)]
pub enum BusError {
    #[error("payload too large for backend: {0} bytes")]
    PayloadTooLarge(usize),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("bus closed")]
    Closed,

    #[error("event log row not found for log_id={log_id}")]
    HydrateNotFound { log_id: i64 },

    #[error("failed to hydrate log_id={log_id}: {reason}")]
    HydrateFailed { log_id: i64, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_hydrate_and_payload_errors() {
        let not_found = BusError::HydrateNotFound { log_id: 42 };
        assert!(not_found.to_string().contains("42"));

        let failed = BusError::HydrateFailed {
            log_id: 7,
            reason: "bad json".into(),
        };
        assert!(failed.to_string().contains("7"));
        assert!(failed.to_string().contains("bad json"));

        let large = BusError::PayloadTooLarge(9000);
        assert!(large.to_string().contains("9000"));
    }
}
