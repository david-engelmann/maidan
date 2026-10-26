//! A thread's digest (C10, summarize once): the newest linked artifact of kind
//! `digest`. One agent writes the summary, uploads it as a `digest` artifact
//! and links it to the thread; every later reader fetches it by sha256.

use maidan_types::{ArtifactKind, ThreadDigest, ThreadId, WorkspaceId};

use crate::{Store, StoreError};

/// The thread's current digest as `workspace_id` holds it, or `None` when no
/// `digest` artifact is linked. Links whose bytes the workspace no longer
/// holds are skipped. The caller has already checked thread access.
pub async fn current(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    thread_id: ThreadId,
) -> Result<Option<ThreadDigest>, StoreError> {
    let links = store.list_thread_artifacts(thread_id).await?;
    for link in links.iter().rev() {
        let artifact = match store
            .get_artifact_for_workspace(workspace_id, &link.sha256)
            .await
        {
            Ok(artifact) => artifact,
            Err(StoreError::NotFound) => continue,
            Err(e) => return Err(e),
        };
        if artifact.kind == ArtifactKind::Digest {
            return Ok(Some(ThreadDigest {
                thread_id,
                uri: format!("maidan://artifacts/{}", link.sha256),
                sha256: link.sha256.clone(),
                size_bytes: artifact.size_bytes,
                mime_type: artifact.mime_type,
                linked_by: link.linked_by,
                linked_at: link.linked_at,
            }));
        }
    }
    Ok(None)
}
