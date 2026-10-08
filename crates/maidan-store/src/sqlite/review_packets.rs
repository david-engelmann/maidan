//! Review packets: what a thread put in front of its reviewers at each
//! `start_review`, pinned by content hash (migration 0145). Written in the
//! transition's own transaction and never updated.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use maidan_types::{
    result_sha256, self_reported_only, EvidenceAttestation, EvidenceManifest, MemberId,
    ResultEvidence, ReviewPacket, ThreadId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::attestation::{attest, Author};
use crate::error::StoreError;

fn row_to_packet(row: &sqlx::sqlite::SqliteRow) -> Result<ReviewPacket, StoreError> {
    Ok(ReviewPacket {
        id: row.get::<Uuid, _>("id"),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        requested_by: MemberId(row.get::<Uuid, _>("requested_by")),
        thread_version: row.get::<i64, _>("thread_version"),
        manifest: serde_json::from_str(&row.get::<String, _>("manifest"))?,
        evidence_root: row.get("evidence_root"),
        self_reported_only: false,
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
    .map(with_warning)
}

/// The server's warning, from the tiers the packet pinned.
fn with_warning(mut packet: ReviewPacket) -> ReviewPacket {
    packet.self_reported_only = self_reported_only(&packet.manifest.attestations);
    packet
}

/// The thread's evidence as it stands in `tx`: its result's hash and its
/// linked artifacts.
pub(crate) async fn manifest_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
) -> Result<EvidenceManifest, StoreError> {
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
        result,
        artifacts,
        attestations: Vec::new(),
    })
}

fn author(member: Uuid, actor: Option<Uuid>) -> Author {
    Author {
        member: MemberId(member),
        actor: actor.map(MemberId),
    }
}

/// The tier of each piece of evidence in `manifest`, judged on the hand-off's
/// transaction from who put it there, who has worked the thread, and the
/// land-gate pass standing now. Pinned in the packet, so nothing that happens
/// after the hand-off moves it.
async fn attestations_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    manifest: &EvidenceManifest,
) -> Result<Vec<EvidenceAttestation>, StoreError> {
    let thread_id = manifest.thread_id;
    let result_author = sqlx::query(
        "SELECT produced_by, produced_actor_id FROM maidan_thread_results WHERE thread_id = ?1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?
    .map(|row| author(row.get("produced_by"), row.get("produced_actor_id")));
    let link_authors: Vec<(String, Author)> = sqlx::query(
        "SELECT sha256, linked_by, linked_actor_id FROM maidan_thread_artifacts
         WHERE thread_id = ?1",
    )
    .bind(thread_id.0)
    .fetch_all(&mut **tx)
    .await?
    .iter()
    .map(|row| {
        (
            row.get::<String, _>("sha256"),
            author(row.get("linked_by"), row.get("linked_actor_id")),
        )
    })
    .collect();
    let workers: HashSet<MemberId> = sqlx::query_scalar::<_, Uuid>(
        "SELECT member_id FROM maidan_thread_workers WHERE thread_id = ?1",
    )
    .bind(thread_id.0)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(MemberId)
    .collect();
    let gate = super::land_gate::read_in_tx(tx, thread_id).await?;
    Ok(attest(
        manifest,
        result_author,
        &link_authors,
        &workers,
        &gate,
    ))
}

/// Record the packet for a `start_review` made in `tx`.
pub(crate) async fn record_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    requested_by: MemberId,
) -> Result<ReviewPacket, StoreError> {
    let thread_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE((SELECT version FROM maidan_thread_versions WHERE thread_id = ?1), 0)",
    )
    .bind(thread_id.0)
    .fetch_one(&mut **tx)
    .await?;
    let mut manifest = manifest_in_tx(tx, thread_id).await?;
    manifest.attestations = attestations_in_tx(tx, &manifest).await?;
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
    .bind(thread_version)
    .bind(serde_json::to_string(&manifest)?)
    .bind(&evidence_root)
    .bind(created_at)
    .execute(&mut **tx)
    .await?;
    Ok(with_warning(ReviewPacket {
        id,
        thread_id,
        requested_by,
        thread_version,
        manifest,
        evidence_root,
        self_reported_only: false,
        created_at,
    }))
}

/// The thread's latest packet: what its current review was handed.
pub async fn latest(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<ReviewPacket>, StoreError> {
    sqlx::query(
        "SELECT id, thread_id, requested_by, thread_version, manifest, evidence_root, created_at
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

/// The root of the thread's latest packet, `None` before any hand-off.
pub(crate) async fn latest_root_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
) -> Result<Option<String>, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT evidence_root FROM maidan_review_packets WHERE thread_id = ?1
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?)
}

/// Refuse a verdict that names evidence other than what the thread's current
/// review was handed, or that the thread has changed since: the reviewer would
/// be deciding on something they were not shown.
pub(crate) async fn verify_root_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    root: &str,
) -> Result<(), StoreError> {
    let Some(latest) = latest_root_in_tx(tx, thread_id).await? else {
        return Err(StoreError::Conflict(
            "nothing was handed to review: there is no evidence to decide on. \
             Next: start_review hands the thread over, then read get_review_packet"
                .into(),
        ));
    };
    if latest != root {
        return Err(StoreError::Conflict(format!(
            "stale evidence: the current review was handed evidence_root {latest}, not {root}. \
             Next: read get_review_packet and decide on what it shows"
        )));
    }
    ensure_unchanged_in_tx(tx, thread_id, &latest).await
}

/// Refuse when the thread's evidence no longer matches what the packet with
/// `root` pinned: its content changed after the hand-off. The evidence is
/// compared, not a recomputed root, because the root also covers the tiers,
/// which were judged once at the hand-off and are not judged again.
pub(crate) async fn ensure_unchanged_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
    root: &str,
) -> Result<(), StoreError> {
    let pinned = sqlx::query(
        "SELECT manifest FROM maidan_review_packets
         WHERE thread_id = ?1 AND evidence_root = ?2
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(thread_id.0)
    .bind(root)
    .fetch_optional(&mut **tx)
    .await?;
    let pinned: Option<EvidenceManifest> = match pinned {
        Some(row) => Some(serde_json::from_str(&row.get::<String, _>("manifest"))?),
        None => None,
    };
    let current = manifest_in_tx(tx, thread_id).await?;
    if !pinned.is_some_and(|pinned| pinned.same_evidence(&current)) {
        return Err(StoreError::Conflict(
            "the evidence changed after it was handed to review: its result or linked artifacts \
             are not what the reviewers were shown. Next: start_review again, so the review \
             covers what is there now"
                .into(),
        ));
    }
    Ok(())
}
