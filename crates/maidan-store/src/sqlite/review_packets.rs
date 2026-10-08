//! Review packets: what a thread put in front of its reviewers at each
//! `start_review`, pinned by content hash (migration 0145). Written in the
//! transition's own transaction and never updated.

use chrono::{DateTime, Utc};
use maidan_types::{
    result_sha256, EvidenceManifest, MemberId, ResultEvidence, ReviewPacket, ThreadId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_packet(row: &sqlx::sqlite::SqliteRow) -> Result<ReviewPacket, StoreError> {
    Ok(ReviewPacket {
        id: row.get::<Uuid, _>("id"),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        requested_by: MemberId(row.get::<Uuid, _>("requested_by")),
        manifest: serde_json::from_str(&row.get::<String, _>("manifest"))?,
        evidence_root: row.get("evidence_root"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}

/// The thread's evidence as it stands in `tx`: its version, its result's hash
/// and its linked artifacts.
pub(crate) async fn manifest_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
) -> Result<EvidenceManifest, StoreError> {
    let thread_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE((SELECT version FROM maidan_thread_versions WHERE thread_id = ?1), 0)",
    )
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?;
    let result = match sqlx::query(
        "SELECT result, produced_by FROM maidan_thread_results WHERE thread_id = ?1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?
    {
        Some(row) => Some(ResultEvidence {
            sha256: result_sha256(&serde_json::from_str::<serde_json::Value>(
                &row.get::<String, _>("result"),
            )?)
            .map_err(|e| StoreError::InvalidInput(e.to_string()))?,
            produced_by: MemberId(row.get::<Uuid, _>("produced_by")),
        }),
        None => None,
    };
    let artifacts: Vec<String> = sqlx::query_scalar(
        "SELECT sha256 FROM maidan_thread_artifacts WHERE thread_id = ?1 ORDER BY sha256",
    )
    .bind(thread_id.0)
    .fetch_all(&mut **tx)
    .await?;
    Ok(EvidenceManifest {
        thread_id,
        thread_version,
        result,
        artifacts,
    })
}

/// Record the packet for a `start_review` made in `tx`.
pub(crate) async fn record_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    requested_by: MemberId,
) -> Result<ReviewPacket, StoreError> {
    let manifest = manifest_in_tx(tx, thread_id).await?;
    let evidence_root = manifest
        .root()
        .map_err(|e| StoreError::InvalidInput(e.to_string()))?;
    let id = Uuid::now_v7();
    let created_at = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_review_packets
            (id, thread_id, requested_by, thread_version, manifest, evidence_root, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(id)
    .bind(thread_id.0)
    .bind(requested_by.0)
    .bind(manifest.thread_version)
    .bind(serde_json::to_string(&manifest)?)
    .bind(&evidence_root)
    .bind(created_at)
    .execute(&mut **tx)
    .await?;
    Ok(ReviewPacket {
        id,
        thread_id,
        requested_by,
        manifest,
        evidence_root,
        created_at,
    })
}

/// The thread's latest packet: what its current review was handed.
pub async fn latest(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<ReviewPacket>, StoreError> {
    sqlx::query(
        "SELECT id, thread_id, requested_by, manifest, evidence_root, created_at
         FROM maidan_review_packets WHERE thread_id = ?1
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    .as_ref()
    .map(row_to_packet)
    .transpose()
}
