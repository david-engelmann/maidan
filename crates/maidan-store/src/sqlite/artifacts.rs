use chrono::{DateTime, Utc};
use maidan_types::{
    Artifact, ArtifactId, ArtifactKind, Event, MemberId, NewArtifact, StoredEvent, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::events;

/// The shared row is content: what the bytes are, never what one workspace
/// said about them. A conflict leaves it as the first upload wrote it; each
/// workspace's own `kind`, `mime_type` and `uploaded_by` live on its ref.
const UPSERT_SQL: &str =
    "INSERT INTO maidan_artifacts (id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT (sha256) DO UPDATE
         SET sha256 = excluded.sha256
     RETURNING id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at, tombstoned_at";

/// An artifact as one workspace sees it: shared content, that workspace's own
/// metadata. A ref written without metadata (older rows, bare access grants)
/// falls back to the shared row for `kind` and `mime_type` — never for
/// `uploaded_by`, which on the shared row may name another tenant's member.
/// An artifact no workspace holds a ref to was uploaded unscoped (bypass, auth
/// disabled); there is no tenant to protect, and it reads as the shared row.
const WORKSPACE_VIEW_SQL: &str = "SELECT a.id, a.sha256, a.size_bytes,
            CASE WHEN r.kind IS NULL THEN a.mime_type ELSE r.mime_type END AS mime_type,
            COALESCE(r.kind, a.kind) AS kind,
            CASE WHEN r.workspace_id IS NULL THEN a.uploaded_by ELSE r.uploaded_by END
                AS uploaded_by,
            COALESCE(r.created_at, a.created_at) AS created_at,
            a.tombstoned_at
     FROM maidan_artifacts a
     LEFT JOIN maidan_artifact_refs r ON r.sha256 = a.sha256 AND r.workspace_id = ?1
     WHERE a.sha256 = ?2
       AND (r.workspace_id IS NOT NULL
            OR NOT EXISTS (SELECT 1 FROM maidan_artifact_refs x WHERE x.sha256 = a.sha256))";

pub async fn upsert(pool: &SqlitePool, new: NewArtifact) -> Result<Artifact, StoreError> {
    let id = Uuid::now_v7();
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

/// Upsert an artifact, optionally record its per-workspace access ref, and
/// append its `ArtifactUpserted` event — all in one transaction `ref_workspace`
/// is `Some` for a non-bypass upload (mirrors the route's `record_artifact_ref`
/// call); the upsert → ref → event ordering is preserved atomically.
pub async fn upsert_with_event(
    pool: &SqlitePool,
    new: NewArtifact,
    ref_workspace: Option<WorkspaceId>,
) -> Result<(Artifact, StoredEvent), StoreError> {
    let id = Uuid::now_v7();
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
    let mut artifact = row_to_artifact(&row)?;
    if let Some(workspace_id) = ref_workspace {
        record_ref_in_tx(&mut tx, workspace_id, &new).await?;
        let row = sqlx::query(WORKSPACE_VIEW_SQL)
            .bind(workspace_id.0)
            .bind(&new.sha256)
            .fetch_one(&mut *tx)
            .await?;
        artifact = row_to_artifact(&row)?;
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

/// Record a per-workspace artifact access ref on a caller-supplied tx — used by
/// `upsert_with_event` so the ref and the event commit atomically.
async fn record_ref_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    workspace_id: WorkspaceId,
    new: &NewArtifact,
) -> Result<(), StoreError> {
    // See the Postgres twin: the workspace's latest upload sets its kind.
    sqlx::query(
        "INSERT INTO maidan_artifact_refs
             (workspace_id, sha256, kind, mime_type, uploaded_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (workspace_id, sha256) DO UPDATE
             SET kind = excluded.kind,
                 mime_type = COALESCE(excluded.mime_type, maidan_artifact_refs.mime_type),
                 uploaded_by = COALESCE(maidan_artifact_refs.uploaded_by, excluded.uploaded_by)",
    )
    .bind(workspace_id.0)
    .bind(&new.sha256)
    .bind(new.kind.as_str())
    .bind(new.mime_type.as_deref())
    .bind(new.uploaded_by.map(|m| m.0))
    .bind(Utc::now())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The artifact as `workspace_id` sees it; `NotFound` without an access ref.
pub async fn get_for_workspace(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    sha256: &str,
) -> Result<Artifact, StoreError> {
    let row = sqlx::query(WORKSPACE_VIEW_SQL)
        .bind(workspace_id.0)
        .bind(sha256)
        .fetch_optional(pool)
        .await?
        .ok_or(StoreError::NotFound)?;
    row_to_artifact(&row)
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
