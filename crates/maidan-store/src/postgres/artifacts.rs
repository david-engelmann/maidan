use chrono::{DateTime, Utc};
use maidan_types::{
    Artifact, ArtifactErasure, ArtifactId, ArtifactKind, Event, MemberId, NewArtifact, StoredEvent,
    WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::postgres::events;

/// The shared row is content: what the bytes are, never what one workspace
/// said about them. A conflict leaves it as the first upload wrote it; each
/// workspace's own `kind`, `mime_type`, `filename` and `uploaded_by` live on
/// its ref.
const UPSERT_SQL: &str =
    "INSERT INTO maidan_artifacts (id, sha256, size_bytes, mime_type, filename, kind, uploaded_by)
     VALUES ($1, $2, $3, $4, $5, $6, $7)
     ON CONFLICT (sha256) DO UPDATE
         SET sha256 = EXCLUDED.sha256
     RETURNING id, sha256, size_bytes, mime_type, filename, kind, uploaded_by, created_at";

/// An artifact as one workspace sees it: shared content, that workspace's own
/// metadata. A ref written without metadata (older rows, bare access grants)
/// falls back to the shared row for `kind` and `mime_type` — never for
/// `uploaded_by` or `filename`, which on the shared row may be another
/// tenant's.
/// An artifact no workspace holds a ref to was uploaded unscoped (bypass, auth
/// disabled); there is no tenant to protect, and it reads as the shared row.
const WORKSPACE_VIEW_SQL: &str = "SELECT a.id, a.sha256, a.size_bytes,
            CASE WHEN r.kind IS NULL THEN a.mime_type ELSE r.mime_type END AS mime_type,
            CASE WHEN r.workspace_id IS NULL THEN a.filename ELSE r.filename END AS filename,
            COALESCE(r.kind, a.kind) AS kind,
            CASE WHEN r.workspace_id IS NULL THEN a.uploaded_by ELSE r.uploaded_by END
                AS uploaded_by,
            COALESCE(r.created_at, a.created_at) AS created_at
     FROM maidan_artifacts a
     LEFT JOIN maidan_artifact_refs r ON r.sha256 = a.sha256 AND r.workspace_id = $1
     WHERE a.sha256 = $2
       AND (r.workspace_id IS NOT NULL
            OR NOT EXISTS (SELECT 1 FROM maidan_artifact_refs x WHERE x.sha256 = a.sha256))";

/// Serialize artifact-row writes for `sha256` with [`reap_blob`]'s claim until
/// the transaction ends. A 64-bit hash of the sha keys the lock; a collision
/// only serializes two unrelated shas.
async fn lock_sha(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    sha256: &str,
) -> Result<(), StoreError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('maidan.artifact:' || $1, 0))")
        .bind(sha256)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The transaction an artifact-row write for `sha256` runs in, begun once no
/// reap holds the sha: the sha is locked, then its lease checked, so a reap
/// cannot claim it between the check and the write.
async fn begin_sha_write<'a>(
    pool: &'a PgPool,
    sha256: &str,
) -> Result<sqlx::Transaction<'a, sqlx::Postgres>, StoreError> {
    loop {
        let mut tx = pool.begin().await?;
        lock_sha(&mut tx, sha256).await?;
        let reaping: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM maidan_artifact_reaps
                           WHERE sha256 = $1 AND expires_at > clock_timestamp())",
        )
        .bind(sha256)
        .fetch_one(&mut *tx)
        .await?;
        if !reaping {
            return Ok(tx);
        }
        tx.rollback().await?;
        tokio::time::sleep(crate::REAP_POLL).await;
    }
}

pub async fn upsert(pool: &PgPool, new: NewArtifact) -> Result<Artifact, StoreError> {
    let id = Uuid::now_v7();
    let mut tx = begin_sha_write(pool, &new.sha256).await?;
    let row = sqlx::query(UPSERT_SQL)
        .bind(id)
        .bind(&new.sha256)
        .bind(new.size_bytes)
        .bind(new.mime_type.as_deref())
        .bind(new.filename.as_deref())
        .bind(new.kind.as_str())
        .bind(new.uploaded_by.map(|m| m.0))
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    row_to_artifact(&row)
}

/// Delete `sha256`'s bytes if no artifact row holds it. See
/// `ArtifactMetaStore::reap_artifact_blob` and the SQLite twin.
///
/// The check and the lease are one short transaction under the sha's lock; the
/// delete runs outside it, so no connection or lock is held while the blob
/// store works. Every artifact-row write for the sha waits while the lease is
/// live.
pub async fn reap_blob(
    pool: &PgPool,
    sha256: &str,
    delete: crate::BlobDelete<'_>,
) -> Result<crate::BlobReap, StoreError> {
    let token = Uuid::now_v7();
    let mut tx = begin_sha_write(pool, sha256).await?;
    let held: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM maidan_artifacts WHERE sha256 = $1)")
            .bind(sha256)
            .fetch_one(&mut *tx)
            .await?;
    if held {
        // Roll back before returning. Dropping the transaction only queues the
        // rollback, so the sha lock can outlive this call. SQLite's twin hits
        // "database is locked" when the next write starts on another connection.
        tx.rollback().await?;
        return Ok(crate::BlobReap::Referenced);
    }
    // A lapsed lease left by a reaper that died is replaced.
    sqlx::query(
        "INSERT INTO maidan_artifact_reaps (sha256, token, expires_at)
         VALUES ($1, $2, clock_timestamp() + make_interval(secs => $3))
         ON CONFLICT (sha256) DO UPDATE
             SET token = EXCLUDED.token, expires_at = EXCLUDED.expires_at",
    )
    .bind(sha256)
    .bind(token)
    .bind(crate::REAP_LEASE_SECS as f64)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let Some(reaped) = crate::run_blob_delete(delete).await else {
        return Ok(crate::abandoned_delete());
    };
    sqlx::query("DELETE FROM maidan_artifact_reaps WHERE sha256 = $1 AND token = $2")
        .bind(sha256)
        .bind(token)
        .execute(pool)
        .await?;
    Ok(reaped)
}

/// Upsert an artifact, optionally record its per-workspace access ref, and
/// append its `ArtifactUpserted` event — all in one transaction — see the
/// SQLite twin.
pub async fn upsert_with_event(
    pool: &PgPool,
    new: NewArtifact,
    ref_workspace: Option<WorkspaceId>,
) -> Result<(Artifact, StoredEvent), StoreError> {
    let id = Uuid::now_v7();
    let mut tx = begin_sha_write(pool, &new.sha256).await?;
    let row = sqlx::query(UPSERT_SQL)
        .bind(id)
        .bind(&new.sha256)
        .bind(new.size_bytes)
        .bind(new.mime_type.as_deref())
        // A scoped upload's name is that workspace's, and stays on its ref.
        .bind(new.filename.as_deref().filter(|_| ref_workspace.is_none()))
        .bind(new.kind.as_str())
        .bind(new.uploaded_by.map(|m| m.0))
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

pub async fn get_by_sha(pool: &PgPool, sha256: &str) -> Result<Artifact, StoreError> {
    let row = sqlx::query(
        "SELECT id, sha256, size_bytes, mime_type, filename, kind, uploaded_by, created_at
         FROM maidan_artifacts WHERE sha256 = $1",
    )
    .bind(sha256)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_artifact(&row)
}

pub async fn record_ref(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    sha256: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_artifact_refs (workspace_id, sha256) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(workspace_id.0)
    .bind(sha256)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a per-workspace artifact access ref on a caller-supplied tx — used by
/// `upsert_with_event` so the ref and the event commit atomically.
async fn record_ref_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    workspace_id: WorkspaceId,
    new: &NewArtifact,
) -> Result<(), StoreError> {
    // The workspace's latest upload sets its kind (and its type and name, when
    // given) and restarts its media TTL; who first uploaded it here, and when,
    // stay.
    sqlx::query(
        "INSERT INTO maidan_artifact_refs
             (workspace_id, sha256, kind, mime_type, filename, uploaded_by, last_uploaded_at)
         VALUES ($1, $2, $3, $4, $5, $6, NOW())
         ON CONFLICT (workspace_id, sha256) DO UPDATE
             SET kind = EXCLUDED.kind,
                 last_uploaded_at = EXCLUDED.last_uploaded_at,
                 mime_type = COALESCE(EXCLUDED.mime_type, maidan_artifact_refs.mime_type),
                 filename = COALESCE(EXCLUDED.filename, maidan_artifact_refs.filename),
                 uploaded_by = COALESCE(maidan_artifact_refs.uploaded_by, EXCLUDED.uploaded_by)",
    )
    .bind(workspace_id.0)
    .bind(&new.sha256)
    .bind(new.kind.as_str())
    .bind(new.mime_type.as_deref())
    .bind(new.filename.as_deref())
    .bind(new.uploaded_by.map(|m| m.0))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The artifact as `workspace_id` sees it; `NotFound` without an access ref.
pub async fn get_for_workspace(
    pool: &PgPool,
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
    pool: &PgPool,
    workspace_id: WorkspaceId,
    sha256: &str,
) -> Result<bool, StoreError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM maidan_artifact_refs WHERE workspace_id = $1 AND sha256 = $2)",
    )
    .bind(workspace_id.0)
    .bind(sha256)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

fn row_to_artifact(row: &sqlx::postgres::PgRow) -> Result<Artifact, StoreError> {
    let kind: String = row.get("kind");
    let kind = ArtifactKind::parse(&kind).ok_or_else(|| {
        StoreError::InvalidInput(format!("unknown artifact kind in database: {kind}"))
    })?;
    Ok(Artifact {
        id: ArtifactId(row.get::<Uuid, _>("id")),
        sha256: row.get("sha256"),
        size_bytes: row.get("size_bytes"),
        mime_type: row.get("mime_type"),
        filename: row.get("filename"),
        kind,
        uploaded_by: row.get::<Option<Uuid>, _>("uploaded_by").map(MemberId),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}

/// Erase `workspace_id`'s reference to the artifact `sha256`, and the artifact
/// row with it when no other workspace still references it, auditing in the
/// same transaction. `NotFound` without a reference; `Conflict` under a legal
/// hold. The caller deletes the blob when `last_reference` is set.
pub async fn erase_for_workspace(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    sha256: &str,
    audit_for: crate::AuditFor<ArtifactErasure>,
) -> Result<ArtifactErasure, StoreError> {
    let mut tx = pool.begin().await?;
    super::legal_hold::refuse_if_held(&mut tx, workspace_id).await?;
    let erasure = erase_ref_in_tx(&mut tx, workspace_id, sha256, audit_for).await?;
    tx.commit().await?;
    Ok(erasure)
}

/// The erase itself, on a transaction that has already refused a held
/// workspace. Shared by an erase on request and a media expiry, so both write
/// the same rows.
async fn erase_ref_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    workspace_id: WorkspaceId,
    sha256: &str,
    audit_for: crate::AuditFor<ArtifactErasure>,
) -> Result<ArtifactErasure, StoreError> {
    // Lock the shared row first, so an upload of the same bytes racing this
    // erase either lands its reference before the check below or waits for it.
    sqlx::query("SELECT 1 FROM maidan_artifacts WHERE sha256 = $1 FOR UPDATE")
        .bind(sha256)
        .fetch_optional(&mut **tx)
        .await?;
    let removed =
        sqlx::query("DELETE FROM maidan_artifact_refs WHERE workspace_id = $1 AND sha256 = $2")
            .bind(workspace_id.0)
            .bind(sha256)
            .execute(&mut **tx)
            .await?;
    if removed.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    // This workspace's share tickets stop granting the artifact.
    sqlx::query(
        "DELETE FROM maidan_share_ticket_artifacts
         WHERE sha256 = $1
           AND ticket_id IN (SELECT id FROM maidan_share_tickets WHERE workspace_id = $2)",
    )
    .bind(sha256)
    .bind(workspace_id.0)
    .execute(&mut **tx)
    .await?;
    // Threads here stop listing the artifact as evidence; their versions move.
    // Another workspace's links to the same bytes stay.
    sqlx::query(
        "DELETE FROM maidan_thread_artifacts
         WHERE sha256 = $1
           AND thread_id IN (
             SELECT t.id FROM maidan_threads t
             JOIN maidan_channels c ON c.id = t.channel_id
             WHERE c.workspace_id = $2)",
    )
    .bind(sha256)
    .bind(workspace_id.0)
    .execute(&mut **tx)
    .await?;
    // A row nobody references reads as unscoped (readable by any workspace),
    // so the last reference must take the row with it.
    let last_reference = sqlx::query(
        "DELETE FROM maidan_artifacts WHERE sha256 = $1
         AND NOT EXISTS (SELECT 1 FROM maidan_artifact_refs WHERE sha256 = $1)",
    )
    .bind(sha256)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        > 0;
    let erasure = ArtifactErasure {
        workspace_id,
        sha256: sha256.to_string(),
        last_reference,
        occurred_at: Utc::now(),
    };
    super::audit::append_counted(tx, audit_for(&erasure)).await?;
    Ok(erasure)
}

/// Whether the ref `r` has expired as of `$1`: a media kind, older than the
/// cutoff, in a workspace under no legal hold, and referenced by nothing in
/// its workspace. A thread that is not tombstoned linking the sha references
/// it, and so does a message that is not tombstoned naming it in its metadata
/// in any shape `artifact_shas_from_metadata` reads: a string
/// `artifact_sha256` or `sha256`, or an `artifacts` array holding the sha as
/// a string or as an object's string `sha256`. Comparing jsonb to jsonb
/// matches strings only, as that function does.
fn expirable_sql() -> String {
    format!(
        "r.kind IN {kinds}
         AND COALESCE(r.last_uploaded_at, r.created_at) < $1
         AND NOT EXISTS (SELECT 1 FROM maidan_legal_holds h WHERE h.workspace_id = r.workspace_id)
         AND NOT EXISTS (
             SELECT 1 FROM maidan_thread_artifacts ta
             JOIN maidan_threads t ON t.id = ta.thread_id
             JOIN maidan_channels c ON c.id = t.channel_id
             WHERE ta.sha256 = r.sha256
               AND c.workspace_id = r.workspace_id
               AND t.tombstoned_at IS NULL)
         AND NOT EXISTS (
             SELECT 1 FROM maidan_messages m
             JOIN maidan_threads t ON t.id = m.thread_id
             JOIN maidan_channels c ON c.id = t.channel_id
             WHERE c.workspace_id = r.workspace_id
               AND m.tombstoned_at IS NULL
               AND (m.metadata -> 'artifact_sha256' = to_jsonb(r.sha256)
                    OR m.metadata -> 'sha256' = to_jsonb(r.sha256)
                    OR (jsonb_typeof(m.metadata -> 'artifacts') = 'array'
                        AND (m.metadata -> 'artifacts' @> jsonb_build_array(r.sha256)
                             OR m.metadata -> 'artifacts'
                                @> jsonb_build_array(jsonb_build_object('sha256', r.sha256))))))",
        kinds = crate::media_retention::MEDIA_KINDS_SQL,
    )
}

pub async fn media_bytes(pool: &PgPool, workspace_id: WorkspaceId) -> Result<i64, StoreError> {
    // One ref per (workspace, sha) and one row per sha: each sha counts once.
    let used: i64 = sqlx::query_scalar(&format!(
        "SELECT COALESCE(SUM(a.size_bytes), 0)::BIGINT
         FROM maidan_artifact_refs r
         JOIN maidan_artifacts a ON a.sha256 = r.sha256
         WHERE r.workspace_id = $1 AND r.kind IN {kinds}",
        kinds = crate::media_retention::MEDIA_KINDS_SQL,
    ))
    .bind(workspace_id.0)
    .fetch_one(pool)
    .await?;
    Ok(used)
}

pub async fn expirable_media_refs(
    pool: &PgPool,
    scope: crate::media_retention::MediaExpiryScope,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<(WorkspaceId, String)>, StoreError> {
    use crate::media_retention::MediaExpiryScope;
    let (scope_sql, workspace) = match scope {
        MediaExpiryScope::Workspace(workspace_id) => ("r.workspace_id = $3", Some(workspace_id.0)),
        MediaExpiryScope::InstanceDefault => (
            "$3::uuid IS NULL AND NOT EXISTS (
                 SELECT 1 FROM maidan_retention_policies p
                 WHERE p.workspace_id = r.workspace_id AND p.media_days IS NOT NULL)",
            None,
        ),
    };
    let rows: Vec<(Uuid, String)> = sqlx::query_as(&format!(
        "SELECT r.workspace_id, r.sha256 FROM maidan_artifact_refs r
         WHERE {scope_sql} AND {expirable}
         ORDER BY COALESCE(r.last_uploaded_at, r.created_at), r.workspace_id, r.sha256
         LIMIT $2",
        expirable = expirable_sql(),
    ))
    .bind(cutoff)
    .bind(limit)
    .bind(workspace)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(workspace_id, sha256)| (WorkspaceId(workspace_id), sha256))
        .collect())
}

pub async fn expire_media_ref(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    sha256: &str,
    cutoff: DateTime<Utc>,
    audit_for: crate::AuditFor<ArtifactErasure>,
) -> Result<Option<ArtifactErasure>, StoreError> {
    let mut tx = pool.begin().await?;
    match super::legal_hold::refuse_if_held(&mut tx, workspace_id).await {
        Ok(()) => {}
        Err(StoreError::Conflict(_) | StoreError::NotFound) => return Ok(None),
        Err(err) => return Err(err),
    }
    let expirable: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS(SELECT 1 FROM maidan_artifact_refs r
                       WHERE r.workspace_id = $2 AND r.sha256 = $3 AND {expirable})",
        expirable = expirable_sql(),
    ))
    .bind(cutoff)
    .bind(workspace_id.0)
    .bind(sha256)
    .fetch_one(&mut *tx)
    .await?;
    if !expirable {
        tx.rollback().await?;
        return Ok(None);
    }
    let erasure = erase_ref_in_tx(&mut tx, workspace_id, sha256, audit_for).await?;
    tx.commit().await?;
    Ok(Some(erasure))
}
