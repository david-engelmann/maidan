use chrono::{DateTime, Utc};
use maidan_types::{
    Artifact, ArtifactId, ArtifactKind, Event, MemberId, NewArtifact, StoredEvent, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::events;

const UPSERT_SQL: &str =
    "INSERT INTO maidan_artifacts (id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT (sha256) DO UPDATE
         SET mime_type = COALESCE(excluded.mime_type, maidan_artifacts.mime_type),
             kind = excluded.kind
     RETURNING id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at, tombstoned_at";

pub async fn upsert(pool: &SqlitePool, new: NewArtifact) -> Result<Artifact, StoreError> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let row = sqlx::query(UPSERT_SQL)
        .bind(id)
        .bind(&new.sha256)
        .bind(new.size_bytes)
        .bind(new.mime_type.as_deref())
        .bind(new.kind.as_str())
        .bind(new.uploaded_by.map(|m| m.0))
        .bind(now)
        .fetch_one(pool)
        .await?;
    row_to_artifact(&row)
}

/// Upsert an artifact, optionally record its per-workspace access ref (Cluster
/// 204), and append its `ArtifactUpserted` event — all in one transaction
/// (Cluster 214). `ref_workspace` is `Some` for a non-bypass upload (mirrors the
/// route's `record_artifact_ref` call); the upsert → ref → event ordering is
/// preserved atomically.
pub async fn upsert_with_event(
    pool: &SqlitePool,
    new: NewArtifact,
    ref_workspace: Option<WorkspaceId>,
) -> Result<(Artifact, StoredEvent), StoreError> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let mut tx = pool.begin().await?;
    let row = sqlx::query(UPSERT_SQL)
        .bind(id)
        .bind(&new.sha256)
        .bind(new.size_bytes)
        .bind(new.mime_type.as_deref())
        .bind(new.kind.as_str())
        .bind(new.uploaded_by.map(|m| m.0))
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
    let artifact = row_to_artifact(&row)?;
    if let Some(workspace_id) = ref_workspace {
        record_ref_in_tx(&mut tx, workspace_id, &artifact.sha256).await?;
    }
    let event = Event::ArtifactUpserted {
        occurred_at: Utc::now(),
        artifact: artifact.clone(),
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((artifact, stored))
}

pub async fn get_by_sha(pool: &SqlitePool, sha256: &str) -> Result<Artifact, StoreError> {
    let row = sqlx::query(
        "SELECT id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at, tombstoned_at
         FROM maidan_artifacts WHERE sha256 = ?",
    )
    .bind(sha256)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_artifact(&row)
}

pub async fn record_ref(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    sha256: &str,
) -> Result<(), StoreError> {
    sqlx::query("INSERT OR IGNORE INTO maidan_artifact_refs (workspace_id, sha256) VALUES (?, ?)")
        .bind(workspace_id.0)
        .bind(sha256)
        .execute(pool)
        .await?;
    Ok(())
}

/// Record a per-workspace artifact access ref on a caller-supplied tx (Cluster
/// 214) — used by `upsert_with_event` so the ref and the event commit atomically.
async fn record_ref_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    workspace_id: WorkspaceId,
    sha256: &str,
) -> Result<(), StoreError> {
    sqlx::query("INSERT OR IGNORE INTO maidan_artifact_refs (workspace_id, sha256) VALUES (?, ?)")
        .bind(workspace_id.0)
        .bind(sha256)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn ref_exists(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    sha256: &str,
) -> Result<bool, StoreError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM maidan_artifact_refs WHERE workspace_id = ? AND sha256 = ?)",
    )
    .bind(workspace_id.0)
    .bind(sha256)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

fn row_to_artifact(row: &sqlx::sqlite::SqliteRow) -> Result<Artifact, StoreError> {
    let kind: String = row.get("kind");
    let kind = ArtifactKind::parse(&kind).ok_or_else(|| {
        StoreError::InvalidInput(format!("unknown artifact kind in database: {kind}"))
    })?;
    Ok(Artifact {
        id: ArtifactId(row.get::<Uuid, _>("id")),
        sha256: row.get("sha256"),
        size_bytes: row.get("size_bytes"),
        mime_type: row.get("mime_type"),
        kind,
        uploaded_by: row.get::<Option<Uuid>, _>("uploaded_by").map(MemberId),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    })
}
