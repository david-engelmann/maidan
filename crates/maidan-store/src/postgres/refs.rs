use chrono::{DateTime, Utc};
use maidan_types::{Event, NewReference, RefSide, Reference, StoredEvent};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::postgres::events;

fn map_ref_err(err: sqlx::Error) -> StoreError {
    match err {
        sqlx::Error::Database(ref db) if db.is_unique_violation() => {
            StoreError::Conflict("reference already exists".into())
        }
        other => StoreError::Database(other),
    }
}

pub async fn create(pool: &PgPool, new: NewReference) -> Result<Reference, StoreError> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO maidan_references (id, src_kind, src_id, dst_kind, dst_id, relation)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, src_kind, src_id, dst_kind, dst_id, relation, created_at",
    )
    .bind(id)
    .bind(new.src_kind.as_str())
    .bind(new.src_id)
    .bind(new.dst_kind.as_str())
    .bind(new.dst_id)
    .bind(&new.relation)
    .fetch_one(pool)
    .await
    .map_err(map_ref_err)?;
    row_to_reference(&row)
}

/// Insert a reference and append its `ReferenceAdded` event in one transaction
/// (Cluster 214) — see the SQLite twin.
pub async fn create_with_event(
    pool: &PgPool,
    new: NewReference,
) -> Result<(Reference, StoredEvent), StoreError> {
    let id = Uuid::new_v4();
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO maidan_references (id, src_kind, src_id, dst_kind, dst_id, relation)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, src_kind, src_id, dst_kind, dst_id, relation, created_at",
    )
    .bind(id)
    .bind(new.src_kind.as_str())
    .bind(new.src_id)
    .bind(new.dst_kind.as_str())
    .bind(new.dst_id)
    .bind(&new.relation)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_ref_err)?;
    let reference = row_to_reference(&row)?;
    let event = Event::ReferenceAdded {
        occurred_at: Utc::now(),
        reference: reference.clone(),
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((reference, stored))
}

pub async fn list_from(
    pool: &PgPool,
    src_kind: RefSide,
    src_id: Uuid,
) -> Result<Vec<Reference>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, src_kind, src_id, dst_kind, dst_id, relation, created_at
         FROM maidan_references
         WHERE src_kind = $1 AND src_id = $2
         ORDER BY created_at ASC",
    )
    .bind(src_kind.as_str())
    .bind(src_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_reference).collect()
}

pub async fn list_from_many(
    pool: &PgPool,
    src_kind: RefSide,
    src_ids: &[Uuid],
) -> Result<Vec<Reference>, StoreError> {
    if src_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT id, src_kind, src_id, dst_kind, dst_id, relation, created_at
         FROM maidan_references
         WHERE src_kind = $1 AND src_id = ANY($2)
         ORDER BY src_id, created_at ASC",
    )
    .bind(src_kind.as_str())
    .bind(src_ids)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_reference).collect()
}

fn parse_side(s: &str) -> Result<RefSide, StoreError> {
    match s {
        "thread" => Ok(RefSide::Thread),
        "message" => Ok(RefSide::Message),
        other => Err(StoreError::InvalidInput(format!(
            "unknown ref side: {other}"
        ))),
    }
}

fn row_to_reference(row: &sqlx::postgres::PgRow) -> Result<Reference, StoreError> {
    let src_kind = parse_side(row.get::<&str, _>("src_kind"))?;
    let dst_kind = parse_side(row.get::<&str, _>("dst_kind"))?;
    Ok(Reference {
        id: row.get::<Uuid, _>("id"),
        src_kind,
        src_id: row.get::<Uuid, _>("src_id"),
        dst_kind,
        dst_id: row.get::<Uuid, _>("dst_id"),
        relation: row.get("relation"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}
