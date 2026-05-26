//! Pluggable embedding providers for the indexer (v1.2.1+).
//!
//! Default: [`HashV1Provider`] (`hash-v1`). Select via `MAIDAN_EMBEDDING_PROVIDER`.

use std::sync::Arc;

use thiserror::Error;

use crate::embeddings::{hash_embedding, model_name as hash_v1_model};
use crate::postgres::EMBEDDING_DIM;

#[derive(Debug, Error)]
pub enum EmbeddingProviderError {
    #[error("unknown embedding provider {name:?}; supported: hash-v1")]
    Unknown { name: String },
}

/// Generates a fixed-dimension vector for message body text.
pub trait EmbeddingProvider: Send + Sync {
    fn model_name(&self) -> &'static str;
    fn dimension(&self) -> usize;
    fn embed(&self, body: &str) -> Vec<f32>;
}

/// SHA-256–derived pseudo-embedding (1024-d, normalized).
pub struct HashV1Provider;

impl EmbeddingProvider for HashV1Provider {
    fn model_name(&self) -> &'static str {
        hash_v1_model()
    }

    fn dimension(&self) -> usize {
        EMBEDDING_DIM
    }

    fn embed(&self, body: &str) -> Vec<f32> {
        hash_embedding(body)
    }
}

/// Resolve a provider by name (`hash-v1` is the only built-in today).
pub fn provider_from_name(
    name: &str,
) -> Result<Arc<dyn EmbeddingProvider>, EmbeddingProviderError> {
    match name.trim() {
        "" | "hash-v1" => Ok(Arc::new(HashV1Provider)),
        other => Err(EmbeddingProviderError::Unknown {
            name: other.to_string(),
        }),
    }
}

/// `MAIDAN_EMBEDDING_PROVIDER` (default `hash-v1`).
pub fn provider_from_env() -> Result<Arc<dyn EmbeddingProvider>, EmbeddingProviderError> {
    let name = std::env::var("MAIDAN_EMBEDDING_PROVIDER").unwrap_or_else(|_| "hash-v1".into());
    provider_from_name(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_v1_provider_matches_legacy_helpers() {
        let p = HashV1Provider;
        assert_eq!(p.model_name(), hash_v1_model());
        assert_eq!(p.dimension(), EMBEDDING_DIM);
        assert_eq!(p.embed("hello"), hash_embedding("hello"));
    }

    #[test]
    fn provider_from_name_defaults_to_hash_v1() {
        let p = provider_from_name("hash-v1").expect("hash-v1");
        assert_eq!(p.model_name(), "hash-v1");
    }
}
