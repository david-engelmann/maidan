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

/// Embed `body` on Tokio's blocking pool. `embed` is synchronous, and a remote
/// provider waits on the network for up to its timeout; called on an async
/// worker, that wait holds the worker and every task queued behind it, so one
/// slow endpoint stalls unrelated requests. Async callers go through here.
pub async fn embed_off_worker(
    provider: &Arc<dyn EmbeddingProvider>,
    body: &str,
) -> Result<Vec<f32>, EmbeddingProviderError> {
    let provider = Arc::clone(provider);
    let body = body.to_owned();
    join_blocking(tokio::task::spawn_blocking(move || provider.embed(&body))).await
}

/// [`embed_off_worker`] for a batch, for the same reason.
pub async fn embed_batch_off_worker(
    provider: &Arc<dyn EmbeddingProvider>,
    bodies: Vec<String>,
) -> Result<Vec<Vec<f32>>, EmbeddingProviderError> {
    let provider = Arc::clone(provider);
    join_blocking(tokio::task::spawn_blocking(move || {
        let refs: Vec<&str> = bodies.iter().map(String::as_str).collect();
        provider.embed_batch(&refs)
    }))
    .await
}

/// A provider panic stays a panic in the caller, as it was when the call ran
/// inline; the task is only cancelled when the runtime shuts down.
async fn join_blocking<T>(
    task: tokio::task::JoinHandle<Result<T, EmbeddingProviderError>>,
) -> Result<T, EmbeddingProviderError> {
    match task.await {
        Ok(result) => result,
        Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
        Err(err) => Err(EmbeddingProviderError::Remote(format!(
            "embedding task did not finish: {err}"
        ))),
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
        let configured_dim = std::env::var("MAIDAN_EMBEDDING_DIM")
            .ok()
            .map(|v| {
                v.parse::<usize>()
                    .map_err(|e| EmbeddingProviderError::InvalidConfig(e.to_string()))
            })
            .transpose()?;
        let timeout_secs = std::env::var("MAIDAN_EMBEDDING_TIMEOUT_SECS")
            .ok()
            .map(|v| {
                v.parse::<u64>()
                    .map_err(|e| EmbeddingProviderError::InvalidConfig(e.to_string()))
            })
            .transpose()?
            .unwrap_or(15);
        Self::new(endpoint, model, api_key, configured_dim, timeout_secs)
    }

    /// The provider for `endpoint` and `model`, probing the endpoint once for
    /// the dimension unless `configured_dim` gives it.
    pub fn new(
        endpoint: String,
        model: String,
        api_key: Option<String>,
        configured_dim: Option<usize>,
        timeout_secs: u64,
    ) -> Result<Self, EmbeddingProviderError> {
        let client = off_runtime(|| {
            Client::builder()
                .timeout(Duration::from_secs(timeout_secs))
                .build()
        })
        .map_err(|e| EmbeddingProviderError::InvalidConfig(e.to_string()))?;
        // Dimension is set explicitly via MAIDAN_EMBEDDING_DIM, or auto-detected
        // by a one-shot probe against the endpoint. Probing here means a
        // mis-configured model or unreachable endpoint fails at boot, not on
        // every message. `provider` carries a placeholder dim only for the probe.
        let provider = Self {
            endpoint,
            model,
            api_key,
            dimension: configured_dim.unwrap_or(0),
            client,
        };
        let dimension = resolve_dimension(configured_dim, || provider.probe_dimension())?;
        Ok(Self {
            dimension,
            ..provider
        })
    }

    /// POST `{model, input}` to the endpoint and parse the response. `input` is
    /// a single string, an array of strings, or the probe sentinel.
    fn request_embeddings(
        &self,
        input: serde_json::Value,
    ) -> Result<OpenAiEmbeddingsResponse, EmbeddingProviderError> {
        let mut req = self
            .client
            .post(&self.endpoint)
            .json(&json!({ "model": self.model, "input": input }));
        if let Some(token) = &self.api_key {
            req = req.bearer_auth(token);
        }
        off_runtime(move || {
            req.send()
                .map_err(|e| EmbeddingProviderError::Remote(e.to_string()))?
                .error_for_status()
                .map_err(|e| EmbeddingProviderError::Remote(e.to_string()))?
                .json()
                .map_err(|e| EmbeddingProviderError::Remote(e.to_string()))
        })
    }

    /// One sentinel embed to learn the model's output dimension (used when
    /// `MAIDAN_EMBEDDING_DIM` is unset).
    fn probe_dimension(&self) -> Result<usize, EmbeddingProviderError> {
        let resp = self.request_embeddings(json!("dimension probe")).map_err(|e| {
            EmbeddingProviderError::InvalidConfig(format!(
                "auto-detect embedding dimension via {} failed (set MAIDAN_EMBEDDING_DIM to skip the probe): {e}",
                self.endpoint
            ))
        })?;
        let dim = resp.data.first().map(|d| d.embedding.len()).unwrap_or(0);
        if dim == 0 {
            return Err(EmbeddingProviderError::InvalidConfig(format!(
                "dimension probe against {} returned no usable embedding",
                self.endpoint
            )));
        }
        Ok(dim)
    }
}

/// Run `f` where `reqwest::blocking` is allowed to block. Its client panics if
/// it is built or sends from inside a Tokio runtime, and this provider is
/// built in the server's async `serve` and called from async search, indexing
/// and reindex paths. A scoped OS thread is outside any runtime, and starting
/// one costs nothing next to a remote embedding call. The join is a blocking
/// wait, so this avoids reqwest's in-runtime panic but still holds the calling
/// worker until the request returns. Async callers keep the worker free by going
/// through [`embed_off_worker`], which runs the whole call on the blocking pool.
fn off_runtime<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    if tokio::runtime::Handle::try_current().is_err() {
        return f();
    }
    std::thread::scope(|scope| scope.spawn(f).join())
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// Resolve the embedding dimension: an explicit positive `configured` value
/// wins; otherwise `probe` is called. A configured `0` is rejected.
fn resolve_dimension(
    configured: Option<usize>,
    probe: impl FnOnce() -> Result<usize, EmbeddingProviderError>,
) -> Result<usize, EmbeddingProviderError> {
    match configured {
        Some(0) => Err(EmbeddingProviderError::InvalidConfig(
            "MAIDAN_EMBEDDING_DIM must be greater than 0".into(),
        )),
        Some(d) => Ok(d),
        None => probe(),
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
        let parsed = self.request_embeddings(json!(body))?;
        let mut batch = parse_embeddings_batch(parsed, self.dimension, 1)?;
        Ok(batch.remove(0))
    }

    fn embed_batch(&self, bodies: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingProviderError> {
        if bodies.is_empty() {
            return Ok(Vec::new());
        }
        let parsed = self.request_embeddings(json!(bodies))?;
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
    fn resolve_dimension_prefers_configured_and_skips_probe() {
        let got = resolve_dimension(Some(1536), || panic!("probe must not run when configured"))
            .expect("configured");
        assert_eq!(got, 1536);
    }

    #[test]
    fn resolve_dimension_probes_when_unset() {
        let got = resolve_dimension(None, || Ok(768)).expect("probed");
        assert_eq!(got, 768);

        let err = resolve_dimension(None, || Err(EmbeddingProviderError::Remote("boom".into())));
        assert!(err.is_err());
    }

    #[test]
    fn resolve_dimension_rejects_zero() {
        assert!(resolve_dimension(Some(0), || Ok(1)).is_err());
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

    /// A one-endpoint stand-in for an OpenAI-compatible server: every request
    /// gets one 3-dimensional embedding, and each connection is closed after
    /// its answer so the client never waits on a reused socket.
    fn stub_endpoint(requests: usize) -> String {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            for stream in listener.incoming().take(requests) {
                let mut stream = stream.expect("accept");
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).expect("header");
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().expect("length");
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).expect("body");
                let reply = r#"{"data":[{"index":0,"embedding":[0.1,0.2,0.3]}]}"#;
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                    reply.len()
                )
                .expect("reply");
            }
        });
        format!("http://{addr}/v1/embeddings")
    }

    // The server builds the provider in its async `serve` and calls it from
    // async handlers; a blocking client used there panics in a debug build.
    #[tokio::test]
    async fn the_remote_provider_builds_probes_and_embeds_inside_a_runtime() {
        let endpoint = stub_endpoint(2);
        let provider =
            OpenAiCompatibleProvider::new(endpoint, "stub".into(), None, None, 5).expect("build");
        assert_eq!(provider.dimension(), 3, "the probe learns the dimension");
        assert_eq!(provider.embed("hello").expect("embed"), vec![0.1, 0.2, 0.3]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_remote_provider_embeds_on_a_multi_thread_runtime() {
        let endpoint = stub_endpoint(1);
        let provider = OpenAiCompatibleProvider::new(endpoint, "stub".into(), None, Some(3), 5)
            .expect("build");
        let batch = provider.embed_batch(&["hello"]).expect("embed");
        assert_eq!(batch, vec![vec![0.1, 0.2, 0.3]]);
    }
}
