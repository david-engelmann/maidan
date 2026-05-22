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
pub use store::Store;
