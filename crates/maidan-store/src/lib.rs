//! Storage abstraction for Maidan.
//!
//! Defines [`Store`], a backend-agnostic async interface, plus Postgres
//! and SQLite implementations backed by `sqlx`.

pub mod dialect;
pub mod error;
pub mod migrate;
pub mod postgres;
pub mod sqlite;
pub mod store;

pub use dialect::Dialect;
pub use error::StoreError;
pub use migrate::{run_postgres_migrations, run_sqlite_migrations};
pub use postgres::PostgresStore;
pub use sqlite::SqliteStore;

/// Applies SQLite PRAGMAs (`foreign_keys`, WAL, `busy_timeout`).
pub async fn configure_sqlite_pool(pool: &sqlx::SqlitePool) -> Result<(), StoreError> {
    sqlite::configure_pool(pool).await
}
pub use store::Store;
