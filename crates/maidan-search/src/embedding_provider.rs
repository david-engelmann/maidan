//! Pluggable embedding providers for indexer + semantic query paths.
//!
//! Default: [`HashV1Provider`] (`hash-v1`). Remote mode:
//! `MAIDAN_EMBEDDING_PROVIDER=openai-compatible`.

use std::{sync::Arc, time::Duration};

use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use crate::embeddings::{hash_embedding, model_name as hash_v1_model};
use crate::postgres::EMBEDDING_DIM;

#[derive(Debug, Error)]
pub enum EmbeddingProviderError {
    #[error("unknown embedding provider {name:?}; supported: hash-v1, openai-compatible")]
    Unknown { name: String },
    #[error("missing required env var {name}")]
    MissingEnv { name: &'static str },
    #[error("invalid embedding provider config: {0}")]
    InvalidConfig(String),
    #[error("remote embedding request failed: {0}")]
    Remote(String),
}

/// Generates a fixed-dimension vector for message body text.
pub trait EmbeddingProvider: Send + Sync {
    fn model_name(&self) -> &str;
    fn dimension(&self) -> usize;
    fn embed(&self, body: &str) -> Result<Vec<f32>, EmbeddingProviderError>;
}

/// SHA-256-derived pseudo-embedding (1024-d, normalized).
pub struct HashV1Provider;

impl EmbeddingProvider for HashV1Provider {
    fn model_name(&self) -> &str {
        hash_v1_model()
    }

    fn dimension(&self) -> usize {
        EMBEDDING_DIM
    }

    fn embed(&self, body: &str) -> Result<Vec<f32>, EmbeddingProviderError> {
        Ok(hash_embedding(body))
    }
}

/// OpenAI-compatible embeddings endpoint.
pub struct OpenAiCompatibleProvider {
    endpoint: String,
    model: String,
    api_key: Option<String>,
    dimension: usize,
    client: Client,
}

impl OpenAiCompatibleProvider {
    fn from_env() -> Result<Self, EmbeddingProviderError> {
        let endpoint = std::env::var("MAIDAN_EMBEDDING_ENDPOINT").map_err(|_| {
            EmbeddingProviderError::MissingEnv {
                name: "MAIDAN_EMBEDDING_ENDPOINT",
            }
        })?;
        let model = std::env::var("MAIDAN_EMBEDDING_MODEL").map_err(|_| {
            EmbeddingProviderError::MissingEnv {
                name: "MAIDAN_EMBEDDING_MODEL",
            }
        })?;
        let api_key = std::env::var("MAIDAN_EMBEDDING_API_KEY").ok();
        let dimension = std::env::var("MAIDAN_EMBEDDING_DIM")
            .ok()
            .map(|v| {
                v.parse::<usize>()
                    .map_err(|e| EmbeddingProviderError::InvalidConfig(e.to_string()))
            })
            .transpose()?
            .unwrap_or(EMBEDDING_DIM);
        let timeout_secs = std::env::var("MAIDAN_EMBEDDING_TIMEOUT_SECS")
            .ok()
            .map(|v| {
                v.parse::<u64>()
                    .map_err(|e| EmbeddingProviderError::InvalidConfig(e.to_string()))
            })
            .transpose()?
            .unwrap_or(15);
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| EmbeddingProviderError::InvalidConfig(e.to_string()))?;
        Ok(Self {
            endpoint,
            model,
            api_key,
            dimension,
            client,
        })
    }
}

impl EmbeddingProvider for OpenAiCompatibleProvider {
    fn model_name(&self) -> &str {
        &self.model
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed(&self, body: &str) -> Result<Vec<f32>, EmbeddingProviderError> {
        let mut req = self
            .client
            .post(&self.endpoint)
            .json(&json!({ "model": self.model, "input": body }));
        if let Some(token) = &self.api_key {
            req = req.bearer_auth(token);
        }
        let resp = req
            .send()
            .map_err(|e| EmbeddingProviderError::Remote(e.to_string()))?
            .error_for_status()
            .map_err(|e| EmbeddingProviderError::Remote(e.to_string()))?;
        let parsed: OpenAiEmbeddingsResponse = resp
            .json()
            .map_err(|e| EmbeddingProviderError::Remote(e.to_string()))?;
        let first = parsed
            .data
            .first()
            .ok_or_else(|| EmbeddingProviderError::Remote("response data is empty".into()))?;
        if first.embedding.len() != self.dimension {
            return Err(EmbeddingProviderError::Remote(format!(
                "expected {}-dim embedding, got {}",
                self.dimension,
                first.embedding.len()
            )));
        }
        Ok(first.embedding.clone())
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingsResponse {
    data: Vec<OpenAiEmbeddingItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingItem {
    embedding: Vec<f32>,
}

/// Resolve a provider by name.
pub fn provider_from_name(
    name: &str,
) -> Result<Arc<dyn EmbeddingProvider>, EmbeddingProviderError> {
    match name.trim() {
        "" | "hash-v1" => Ok(Arc::new(HashV1Provider)),
        "openai-compatible" => Ok(Arc::new(OpenAiCompatibleProvider::from_env()?)),
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
        assert_eq!(p.embed("hello").expect("embed"), hash_embedding("hello"));
    }

    #[test]
    fn provider_from_name_defaults_to_hash_v1() {
        let p = provider_from_name("hash-v1").expect("hash-v1");
        assert_eq!(p.model_name(), "hash-v1");
    }
}
