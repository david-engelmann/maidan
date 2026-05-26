//! Search over Maidan content.
//!
//! Two implementations of [`Search`]:
//!
//! - [`PostgresSearch`] — `tsvector` lexical search with `ts_headline`
//!   snippets. Semantic search via `pgvector` arrives in Cluster C PR #3.
//! - [`SqliteSearch`] — FTS5 lexical search with the `snippet()`
//!   function. Semantic search returns [`SearchError::Unsupported`].

pub mod embedding_handler;
pub mod embedding_provider;
pub mod embeddings;
pub mod error;
pub mod filters;
pub mod hit;
pub mod indexer;
pub mod postgres;
pub mod sqlite;
pub mod traits;

pub use embedding_handler::EmbeddingHandler;
pub use embedding_provider::{
    provider_from_env, provider_from_name, EmbeddingProvider, EmbeddingProviderError,
    HashV1Provider,
};
pub use embeddings::{hash_embedding, model_name};
pub use error::SearchError;
pub use filters::SearchFilters;
pub use hit::SearchHit;
pub use indexer::{EventHandler, Indexer, IndexerHandle, LoggingHandler};
pub use postgres::PostgresSearch;
pub use sqlite::SqliteSearch;
pub use traits::Search;
