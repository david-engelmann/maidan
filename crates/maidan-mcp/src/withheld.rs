//! Why an open task is withheld from claims (#1681), shared by the REST
//! thread read and the MCP thread resource so both say the same thing.

use maidan_auth::AuthContext;
use maidan_store::{Store, StoreError};
use maidan_types::{Thread, ThreadState, ThreadWithheld};

/// Why an open thread is withheld as a live duplicate, or `None`. Mirrors
/// the claim rule: its fingerprint links an original that is still live and
/// open or in review. The original is named only when the caller may read
/// it; the reason shows either way.
pub async fn withheld_as_duplicate(
    store: &dyn Store,
    auth: &AuthContext,
    thread: &Thread,
) -> Result<Option<ThreadWithheld>, StoreError> {
    if thread.state != ThreadState::Open {
        return Ok(None);
    }
    let Some(original) = store
        .get_task_fingerprint(thread.id)
        .await?
        .and_then(|fp| fp.duplicate_of)
    else {
        return Ok(None);
    };
    let live = match store.get_thread(original).await {
        Ok(t) => matches!(t.state, ThreadState::Open | ThreadState::InReview),
        Err(StoreError::NotFound) => false,
        Err(err) => return Err(err),
    };
    if !live {
        return Ok(None);
    }
    let visible = maidan_auth::ensure_thread_access(store, auth, original)
        .await
        .is_ok();
    Ok(Some(ThreadWithheld {
        reason: "duplicate_of".into(),
        thread: visible.then_some(original),
    }))
}
