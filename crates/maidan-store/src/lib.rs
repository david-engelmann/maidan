//! Storage abstraction for Maidan.
//!
//! Defines [`Store`], a backend-agnostic async interface, plus a Postgres
//! implementation backed by `sqlx`. SQLite parity arrives in Cluster A
//! PR #6.

pub mod error;
pub mod migrate;
pub mod postgres;
pub mod store;

pub use error::StoreError;
pub use migrate::run_postgres_migrations;
pub use postgres::PostgresStore;
pub use store::Store;
