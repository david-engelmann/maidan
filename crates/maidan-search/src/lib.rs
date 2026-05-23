//! Search over Maidan content.
//!
//! Two implementations of [`Search`]:
//!
//! - [`PostgresSearch`] — `tsvector` lexical search with `ts_headline`
//!   snippets. Semantic search via `pgvector` arrives in Cluster C PR #3.
//! - [`SqliteSearch`] — FTS5 lexical search with the `snippet()`
//!   function. Semantic search returns [`SearchError::Unsupported`].

pub mod error;
pub mod hit;
pub mod postgres;
pub mod sqlite;
pub mod traits;

pub use error::SearchError;
pub use hit::SearchHit;
pub use postgres::PostgresSearch;
pub use sqlite::SqliteSearch;
pub use traits::Search;
