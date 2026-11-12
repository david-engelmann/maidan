use chrono::{DateTime, Utc};
use maidan_types::{
    Event, MemberId, NewReference, RefSide, Reference, RelationKind, StoredEvent, WorkspaceId,
    DM_CHANNEL_NAME,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::audit_scope;
use crate::error::StoreError;
use crate::reference_rules;
use crate::sqlite::events;
use crate::thread_access::readable_thread;

fn map_ref_err(err: sqlx::Error) -> StoreError {
    match err {
        sqlx::Error::Database(ref db) if db.is_unique_violation() => {
            StoreError::Conflict("reference already exists".into())
        }
        other => StoreError::Database(other),
    }
}

async fn endpoint_workspace(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    kind: RefSide,
    id: Uuid,
) -> Result<Option<WorkspaceId>, StoreError> {
    let sql = match kind {
        RefSide::Thread => audit_scope::THREAD_WORKSPACE,
        RefSide::Message => audit_scope::MESSAGE_WORKSPACE,
    };
    let row = sqlx::query(sql).bind(id).fetch_optional(&mut **tx).await?;
    Ok(row.map(|row| WorkspaceId(row.get::<Uuid, _>("workspace_id"))))
}

async fn cycle_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    new: &NewReference,
) -> Result<bool, StoreError> {
    let hit = sqlx::query(reference_rules::CYCLE_SQL)
        .bind(new.dst_kind.as_str())
        .bind(new.dst_id)
        .bind(new.relation.as_str())
        .bind(new.src_kind.as_str())
        .bind(new.src_id)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(hit.is_some())
}

/// Shared write-time check used by `create`, `create_with_event`, and
/// close-abandoned, so a reference that breaks the rules is not stored.
pub(crate) async fn validate_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    new: &NewReference,
) -> Result<(), StoreError> {
    let src_workspace = endpoint_workspace(tx, new.src_kind, new.src_id).await?;
    let dst_workspace = endpoint_workspace(tx, new.dst_kind, new.dst_id).await?;
    let would_cycle = match (src_workspace, dst_workspace) {
        (Some(src), Some(dst))
            if src == dst && reference_rules::is_acyclic_relation(&new.relation) =>
        {
            cycle_exists(tx, new).await?
        }
        _ => false,
    };
    reference_rules::check(new, src_workspace, dst_workspace, would_cycle)
}

async fn insert_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    new: NewReference,
) -> Result<Reference, StoreError> {
    validate_in_tx(tx, &new).await?;
    let id = Uuid::now_v7();
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_references (id, src_kind, src_id, dst_kind, dst_id, relation, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id, src_kind, src_id, dst_kind, dst_id, relation, created_at",
    )
    .bind(id)
    .bind(new.src_kind.as_str())
    .bind(new.src_id)
    .bind(new.dst_kind.as_str())
    .bind(new.dst_id)
    .bind(new.relation.as_str())
    .bind(now)
    .fetch_one(&mut **tx)
    .await
    .map_err(map_ref_err)?;
    row_to_reference(&row)
}

pub async fn create(pool: &SqlitePool, new: NewReference) -> Result<Reference, StoreError> {
    let mut tx = super::tx::begin_write(pool).await?;
    let reference = insert_in_tx(&mut tx, new).await?;
    tx.commit().await?;
    Ok(reference)
}

/// Insert a reference and append its `ReferenceAdded` event in one transaction.
pub async fn create_with_event(
    pool: &SqlitePool,
    new: NewReference,
) -> Result<(Reference, StoredEvent), StoreError> {
    let mut tx = super::tx::begin_write(pool).await?;
    let reference = insert_in_tx(&mut tx, new).await?;
    let event = Event::ReferenceAdded {
        occurred_at: Utc::now(),
        reference: reference.clone(),
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((reference, stored))
}

pub async fn list_from(
    pool: &SqlitePool,
    src_kind: RefSide,
    src_id: Uuid,
) -> Result<Vec<Reference>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, src_kind, src_id, dst_kind, dst_id, relation, created_at
         FROM maidan_references
         WHERE src_kind = ? AND src_id = ?
         ORDER BY created_at ASC, id ASC",
    )
    .bind(src_kind.as_str())
    .bind(src_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_reference).collect()
}

/// References pointing AT one target — the reverse edge. Uses the existing
/// `idx_references_dst` index. Ordered `created_at ASC`.
pub async fn list_to(
    pool: &SqlitePool,
    dst_kind: RefSide,
    dst_id: Uuid,
) -> Result<Vec<Reference>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, src_kind, src_id, dst_kind, dst_id, relation, created_at
         FROM maidan_references
         WHERE dst_kind = ? AND dst_id = ?
         ORDER BY created_at ASC, id ASC",
    )
    .bind(dst_kind.as_str())
    .bind(dst_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_reference).collect()
}

/// [`list_to`] narrowed to sources `reader_id` may read, filtered in the
/// query: a thread source must itself be readable and a message source's
/// thread must be. `workspace_id` is the target's workspace; read access is
/// judged there, the way `thread_access::readable_thread` defines it.
/// Numbered placeholders: the member placeholder appears three times in the
/// generated SQL but binds once.
pub async fn list_to_for_reader(
    pool: &SqlitePool,
    dst_kind: RefSide,
    dst_id: Uuid,
    workspace_id: WorkspaceId,
    reader_id: MemberId,
) -> Result<Vec<Reference>, StoreError> {
    let readable_thread_src = readable_thread("r.src_id", "?3", "?4", "?5");
    let readable_message_src = readable_thread("sm.thread_id", "?3", "?4", "?5");
    let rows = sqlx::query(&format!(
        "SELECT r.id, r.src_kind, r.src_id, r.dst_kind, r.dst_id, r.relation, r.created_at
         FROM maidan_references r
         WHERE r.dst_kind = ?1 AND r.dst_id = ?2
           AND (
             (r.src_kind = 'thread' AND {readable_thread_src})
             OR (r.src_kind = 'message' AND EXISTS (
                   SELECT 1 FROM maidan_messages sm
                   WHERE sm.id = r.src_id AND {readable_message_src}
                 ))
           )
         ORDER BY r.created_at ASC, r.id ASC"
    ))
    .bind(dst_kind.as_str())
    .bind(dst_id)
    .bind(workspace_id.0)
    .bind(reader_id.0)
    .bind(DM_CHANNEL_NAME)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_reference).collect()
}

/// SQLite has no array binding, so expand `IN (?, …)` and chunk the id set well
/// under the variable limit (one slot is reserved for `src_kind`).
const SQLITE_IN_CHUNK: usize = 400;

pub async fn list_from_many(
    pool: &SqlitePool,
    src_kind: RefSide,
    src_ids: &[Uuid],
) -> Result<Vec<Reference>, StoreError> {
    let mut out = Vec::new();
    for chunk in src_ids.chunks(SQLITE_IN_CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let sql = format!(
            "SELECT id, src_kind, src_id, dst_kind, dst_id, relation, created_at
             FROM maidan_references
             WHERE src_kind = ? AND src_id IN ({placeholders})
             ORDER BY src_id, created_at ASC, id ASC"
        );
        let mut q = sqlx::query(&sql).bind(src_kind.as_str());
        for id in chunk {
            q = q.bind(*id);
        }
        let rows = q.fetch_all(pool).await?;
        for row in &rows {
            out.push(row_to_reference(row)?);
        }
    }
    Ok(out)
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

pub(crate) fn row_to_reference(row: &sqlx::sqlite::SqliteRow) -> Result<Reference, StoreError> {
    let src_kind = parse_side(row.get::<&str, _>("src_kind"))?;
    let dst_kind = parse_side(row.get::<&str, _>("dst_kind"))?;
    Ok(Reference {
        id: row.get::<Uuid, _>("id"),
        src_kind,
        src_id: row.get::<Uuid, _>("src_id"),
        dst_kind,
        dst_id: row.get::<Uuid, _>("dst_id"),
        relation: RelationKind::from_wire(row.get::<String, _>("relation").as_str()),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}
