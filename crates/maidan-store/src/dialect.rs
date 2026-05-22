//! Database dialect detection. Used by the server to choose which
//! concrete `Store` impl + migration runner to instantiate.

use crate::error::StoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    Sqlite,
}

impl Dialect {
    /// Detect the dialect from a connection string. Recognizes
    /// `postgres://`, `postgresql://`, and `sqlite://` prefixes.
    pub fn from_url(url: &str) -> Result<Self, StoreError> {
        if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            Ok(Self::Postgres)
        } else if url.starts_with("sqlite:") {
            Ok(Self::Sqlite)
        } else {
            Err(StoreError::InvalidInput(format!(
                "cannot detect dialect from connection URL '{url}': expected postgres://, postgresql://, or sqlite:"
            )))
        }
    }
}
