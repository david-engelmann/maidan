//! Per-model embedding table registry and DDL helpers (Cluster 47).

use sqlx::{PgPool, SqlitePool};
use thiserror::Error;

use crate::error::SearchError;

/// Default dimension for legacy `hash-v1` and migrations.
pub const DEFAULT_EMBEDDING_DIM: usize = 1024;

#[derive(Debug, Error)]
pub enum EmbeddingTableError {
    #[error("invalid embedding model name")]
    InvalidModel,
    #[error("dimension mismatch for model {model}: registry has {registered}, got {provided}")]
    DimensionMismatch {
        model: String,
        registered: usize,
        provided: usize,
    },
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

impl From<EmbeddingTableError> for SearchError {
    fn from(err: EmbeddingTableError) -> Self {
        SearchError::InvalidQuery(err.to_string())
    }
}

/// SQL-safe table name for a model (`maidan_emb_hash_v1`, etc.).
pub fn table_name_for_model(model: &str) -> Result<String, EmbeddingTableError> {
    let slug = model_slug(model)?;
    Ok(format!("maidan_emb_{slug}"))
}

fn model_slug(model: &str) -> Result<String, EmbeddingTableError> {
    let trimmed = model.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return Err(EmbeddingTableError::InvalidModel);
    }
    let slug: String = trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let slug = slug.trim_matches('_');
    if slug.is_empty() {
        return Err(EmbeddingTableError::InvalidModel);
    }
    Ok(slug.to_string())
}

fn assert_safe_table_name(table: &str) -> Result<(), EmbeddingTableError> {
    if !table.starts_with("maidan_emb_") {
        return Err(EmbeddingTableError::InvalidModel);
    }
    if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(EmbeddingTableError::InvalidModel);
    }
    Ok(())
}

pub async fn ensure_model_postgres(
    pool: &PgPool,
    model: &str,
    dimension: usize,
    hnsw: crate::hnsw::HnswParams,
) -> Result<String, EmbeddingTableError> {
    let table = table_name_for_model(model)?;
    assert_safe_table_name(&table)?;

    let existing: Option<(i32, String)> = sqlx::query_as(
        "SELECT dimension, table_name FROM maidan_embedding_models WHERE model = $1",
    )
    .bind(model)
    .fetch_optional(pool)
    .await?;

    if let Some((registered, registered_table)) = existing {
        if usize::try_from(registered).unwrap_or(0) != dimension {
            return Err(EmbeddingTableError::DimensionMismatch {
                model: model.to_string(),
                registered: registered as usize,
                provided: dimension,
            });
        }
        return Ok(registered_table);
    }

    let ddl = format!(
        "CREATE TABLE IF NOT EXISTS {table} (
            message_id UUID PRIMARY KEY REFERENCES maidan_messages(id) ON DELETE CASCADE,
            embedding vector({dimension}) NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )"
    );
    sqlx::raw_sql(&ddl).execute(pool).await?;

    let idx = format!("idx_{table}_hnsw");
    // Build params (`m` / `ef_construction`) only affect indexes created here;
    // changing them later requires a rebuild (see the reindex job + docs).
    let idx_ddl = format!(
        "CREATE INDEX IF NOT EXISTS {idx} ON {table} USING hnsw (embedding vector_cosine_ops){}",
        hnsw.build_with_clause()
    );
    sqlx::raw_sql(&idx_ddl).execute(pool).await?;

    sqlx::query(
        "INSERT INTO maidan_embedding_models (model, dimension, table_name)
         VALUES ($1, $2, $3)
         ON CONFLICT (model) DO NOTHING",
    )
    .bind(model)
    .bind(i32::try_from(dimension).unwrap_or(i32::MAX))
    .bind(&table)
    .execute(pool)
    .await?;

    Ok(table)
}

pub async fn resolve_table_postgres(
    pool: &PgPool,
    model: &str,
) -> Result<Option<(String, usize)>, EmbeddingTableError> {
    let row: Option<(String, i32)> = sqlx::query_as(
        "SELECT table_name, dimension FROM maidan_embedding_models WHERE model = $1",
    )
    .bind(model)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(t, d)| (t, d as usize)))
}

pub async fn ensure_model_sqlite(
    pool: &SqlitePool,
    model: &str,
    dimension: usize,
) -> Result<String, EmbeddingTableError> {
    let table = table_name_for_model(model)?;
    assert_safe_table_name(&table)?;

    let existing: Option<(i32, String)> =
        sqlx::query_as("SELECT dimension, table_name FROM maidan_embedding_models WHERE model = ?")
            .bind(model)
            .fetch_optional(pool)
            .await?;

    if let Some((registered, registered_table)) = existing {
        if usize::try_from(registered).unwrap_or(0) != dimension {
            return Err(EmbeddingTableError::DimensionMismatch {
                model: model.to_string(),
                registered: registered as usize,
                provided: dimension,
            });
        }
        return Ok(registered_table);
    }

    let ddl = format!(
        "CREATE TABLE IF NOT EXISTS {table} (
            message_id TEXT PRIMARY KEY REFERENCES maidan_messages(id) ON DELETE CASCADE,
            embedding BLOB NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )"
    );
    sqlx::raw_sql(&ddl).execute(pool).await?;

    sqlx::query(
        "INSERT INTO maidan_embedding_models (model, dimension, table_name)
         VALUES (?, ?, ?)
         ON CONFLICT (model) DO NOTHING",
    )
    .bind(model)
    .bind(i32::try_from(dimension).unwrap_or(i32::MAX))
    .bind(&table)
    .execute(pool)
    .await?;

    Ok(table)
}

pub async fn resolve_table_sqlite(
    pool: &SqlitePool,
    model: &str,
) -> Result<Option<(String, usize)>, EmbeddingTableError> {
    let row: Option<(String, i32)> =
        sqlx::query_as("SELECT table_name, dimension FROM maidan_embedding_models WHERE model = ?")
            .bind(model)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(t, d)| (t, d as usize)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_name_sanitizes_model() {
        assert_eq!(
            table_name_for_model("hash-v1").expect("slug"),
            "maidan_emb_hash_v1"
        );
        assert_eq!(
            table_name_for_model("test-model").expect("slug"),
            "maidan_emb_test_model"
        );
    }
}
