//! Media retention and the media quota, backend-neutral: whether an upload
//! fits the workspace's quota, which of a workspace's media artifacts have
//! expired, and erasing those through the same audited path as
//! `DELETE /artifacts/{sha}`.
//!
//! Media (attachments, screenshots, recordings) fills storage fast, and much
//! of it is uploaded and never pointed at. A workspace's media ref expires
//! once it is older than the workspace's media TTL and nothing in that
//! workspace references it: no live thread links it, and no live message
//! names it in its metadata. An artifact a result or a message still lists
//! must not vanish, so a reference keeps it however old. Evidence kinds
//! (transcripts, code dumps, context snapshots) are never expired, and a
//! workspace under legal hold keeps everything.

use chrono::{DateTime, Duration, Utc};
use maidan_types::{
    ArtifactErasure, ArtifactKind, AuditScope, NewAuditEvent, RetentionDays, WorkspaceId,
};

use crate::{Store, StoreError};

/// The media kinds, as SQL. Kept in step with `ArtifactKind::is_media` by a
/// test below.
pub(crate) const MEDIA_KINDS_SQL: &str = "('attachment', 'screenshot', 'recording')";

/// An upload the media quota refuses, with what a client needs to free room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "media quota exceeded: the workspace holds {used_bytes} bytes of media, its quota is \
     {quota_bytes}, and this upload is {upload_bytes}"
)]
pub struct MediaQuotaExceeded {
    pub used_bytes: i64,
    pub quota_bytes: i64,
    pub upload_bytes: i64,
}

/// Whether `workspace_id` may take `upload_bytes` more media as `sha256` of
/// `kind`. `Some` is a refusal. Only media counts; bytes the workspace already
/// references cost nothing, since dedup stores them once. The quota is the
/// workspace's own `media_quota_bytes` held to the instance's; neither set
/// means no quota.
///
/// The check reads usage, then the caller writes: two uploads racing can
/// both pass and overshoot the quota by at most what is in flight. A quota
/// bounds storage, it is not an exact ledger, so that is accepted rather than
/// serializing every media upload in a workspace.
pub async fn check_media_quota(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    instance: &RetentionDays,
    kind: ArtifactKind,
    sha256: &str,
    upload_bytes: i64,
) -> Result<Option<MediaQuotaExceeded>, StoreError> {
    if !kind.is_media() {
        return Ok(None);
    }
    let workspace = store.get_retention_policy(workspace_id).await?;
    let Some(quota_bytes) = workspace.stricter(instance).media_quota_bytes else {
        return Ok(None);
    };
    if store.artifact_ref_exists(workspace_id, sha256).await? {
        return Ok(None);
    }
    let used_bytes = store.media_artifact_bytes(workspace_id).await?;
    if used_bytes.saturating_add(upload_bytes) > quota_bytes {
        return Ok(Some(MediaQuotaExceeded {
            used_bytes,
            quota_bytes,
            upload_bytes,
        }));
    }
    Ok(None)
}

/// Which workspaces' refs one expiry pass covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaExpiryScope {
    /// One workspace that set its own `media_days`.
    Workspace(WorkspaceId),
    /// Every workspace that did not set its own `media_days`: they all expire
    /// media on the instance's TTL.
    InstanceDefault,
}

/// The audit row a sweep writes for each expired ref. A system action: no
/// member did it, so there is no actor, as with the other worker-written rows.
fn expiry_audit(erasure: &ArtifactErasure) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(erasure.workspace_id),
        actor_id: None,
        action: "artifact.expire".into(),
        target_kind: Some("workspace".into()),
        target_id: Some(erasure.workspace_id.0),
        metadata: serde_json::json!({
            "sha256": erasure.sha256,
            "last_reference": erasure.last_reference,
        }),
    }
}

/// A workspace's own media TTL held to the instance's, as the other kinds
/// are: a workspace that set a longer one before the operator lowered the
/// instance's gets the instance's.
fn effective_days(workspace: i64, instance: Option<i64>) -> i64 {
    instance.map_or(workspace, |ceiling| workspace.min(ceiling))
}

/// Erase up to `limit` expired media refs, as of `now`, and return what was
/// erased. Each workspace with its own `media_days` is swept on that; every
/// other workspace on `instance_days` (`None`: the instance does not expire
/// media). The caller deletes the bytes of each erasure with
/// `last_reference` set, and calls again while a full page comes back.
///
/// A candidate is checked again inside its erasing transaction, so a
/// reference made after it was listed keeps it.
pub async fn expire_media_artifacts(
    store: &dyn Store,
    now: DateTime<Utc>,
    instance_days: Option<i64>,
    limit: i64,
) -> Result<Vec<ArtifactErasure>, StoreError> {
    let policies = store.list_retention_policies().await?;
    let mut passes: Vec<(MediaExpiryScope, i64)> = policies
        .iter()
        .filter_map(|(workspace_id, policy): &(WorkspaceId, RetentionDays)| {
            let days = policy.media_days?;
            Some((
                MediaExpiryScope::Workspace(*workspace_id),
                effective_days(days, instance_days),
            ))
        })
        .collect();
    if let Some(days) = instance_days {
        passes.push((MediaExpiryScope::InstanceDefault, days));
    }

    let mut erased = Vec::new();
    for (scope, days) in passes {
        let remaining = limit - erased.len() as i64;
        if remaining <= 0 {
            break;
        }
        let cutoff = now - Duration::days(days);
        let candidates = store
            .expirable_media_artifact_refs(scope, cutoff, remaining)
            .await?;
        for (workspace_id, sha256) in candidates {
            match store
                .expire_media_artifact_audited(
                    workspace_id,
                    &sha256,
                    cutoff,
                    Box::new(expiry_audit),
                )
                .await
            {
                Ok(Some(erasure)) => erased.push(erasure),
                // Referenced, held or erased since it was listed: it stays.
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        workspace_id = %workspace_id.0,
                        sha256,
                        "retention: expiring a media artifact failed"
                    );
                }
            }
        }
    }
    Ok(erased)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_types::ArtifactKind;

    #[test]
    fn the_sql_media_kinds_are_the_kinds_that_are_media() {
        for kind in [
            ArtifactKind::Screenshot,
            ArtifactKind::Recording,
            ArtifactKind::Transcript,
            ArtifactKind::CodeDump,
            ArtifactKind::Attachment,
            ArtifactKind::ContextSnapshot,
        ] {
            assert_eq!(
                MEDIA_KINDS_SQL.contains(&format!("'{}'", kind.as_str())),
                kind.is_media(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_workspace_media_ttl_is_held_to_the_instances() {
        assert_eq!(effective_days(7, Some(30)), 7);
        assert_eq!(effective_days(60, Some(30)), 30);
        assert_eq!(effective_days(60, None), 60);
    }
}
