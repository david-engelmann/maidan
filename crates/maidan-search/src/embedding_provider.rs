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

    /// Embed a batch of bodies in one shot, returning one vector per input
    /// in the same order. Providers backed by a remote API should override
    /// this to issue a single request; the default falls back to per-item
    /// [`embed`](Self::embed) so local providers need no extra code.
    ///
    /// All-or-nothing: any failure returns `Err` for the whole batch and the
    /// caller decides how to account for the dropped items.
    fn embed_batch(&self, bodies: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingProviderError> {
        bodies.iter().map(|b| self.embed(b)).collect()
    }
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
        let mut batch = parse_embeddings_batch(parsed, self.dimension, 1)?;
        Ok(batch.remove(0))
    }

    fn embed_batch(&self, bodies: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingProviderError> {
        if bodies.is_empty() {
            return Ok(Vec::new());
        }
        let mut req = self
            .client
            .post(&self.endpoint)
            .json(&json!({ "model": self.model, "input": bodies }));
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
        parse_embeddings_batch(parsed, self.dimension, bodies.len())
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingsResponse {
    data: Vec<OpenAiEmbeddingItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingItem {
    #[serde(default)]
    index: i64,
    embedding: Vec<f32>,
}

/// Validate and order an OpenAI-compatible embeddings response into one vector
/// per input. The API returns items with an `index`; we sort by it so order
/// matches the request even if the server reorders. Servers that omit `index`
/// (all default to 0) keep request order via the stable sort.
fn parse_embeddings_batch(
    mut resp: OpenAiEmbeddingsResponse,
    dimension: usize,
    expected: usize,
) -> Result<Vec<Vec<f32>>, EmbeddingProviderError> {
    if resp.data.len() != expected {
        return Err(EmbeddingProviderError::Remote(format!(
            "expected {expected} embeddings, got {}",
            resp.data.len()
        )));
    }
    resp.data.sort_by_key(|d| d.index);
    let mut out = Vec::with_capacity(expected);
    for item in resp.data {
        if item.embedding.len() != dimension {
            return Err(EmbeddingProviderError::Remote(format!(
                "expected {dimension}-dim embedding, got {}",
                item.embedding.len()
            )));
        }
        out.push(item.embedding);
    }
    Ok(out)
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

    #[test]
    fn default_embed_batch_matches_per_item_embed() {
        let p = HashV1Provider;
        let bodies = ["alpha", "beta", "gamma"];
        let batch = p.embed_batch(&bodies).expect("batch");
        assert_eq!(batch.len(), 3);
        for (vec, body) in batch.iter().zip(bodies.iter()) {
            assert_eq!(vec, &p.embed(body).expect("embed"));
        }
    }

    #[test]
    fn empty_batch_is_empty() {
        assert!(HashV1Provider.embed_batch(&[]).expect("batch").is_empty());
    }

    fn item(index: i64, v: f32, dim: usize) -> OpenAiEmbeddingItem {
        OpenAiEmbeddingItem {
            index,
            embedding: vec![v; dim],
        }
    }

    #[test]
    fn parse_batch_orders_by_index() {
        let resp = OpenAiEmbeddingsResponse {
            data: vec![item(2, 0.2, 4), item(0, 0.0, 4), item(1, 0.1, 4)],
        };
        let out = parse_embeddings_batch(resp, 4, 3).expect("parse");
        assert_eq!(out[0][0], 0.0);
        assert_eq!(out[1][0], 0.1);
        assert_eq!(out[2][0], 0.2);
    }

    #[test]
    fn parse_batch_rejects_count_and_dimension_mismatch() {
        let short = OpenAiEmbeddingsResponse {
            data: vec![item(0, 0.0, 4)],
        };
        assert!(parse_embeddings_batch(short, 4, 2).is_err());

        let wrong_dim = OpenAiEmbeddingsResponse {
            data: vec![item(0, 0.0, 3)],
        };
        assert!(parse_embeddings_batch(wrong_dim, 4, 1).is_err());
    }
}
