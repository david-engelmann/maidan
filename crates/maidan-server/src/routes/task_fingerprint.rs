//! A task's fingerprint over REST (Context Economics C8; see
//! [`maidan_types::task_fingerprint`]): the hash of its brief and inputs,
//! and the open task with the same fingerprint it was linked to when
//! created.

use axum::{extract::State, http::StatusCode, Extension, Json};
use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
use maidan_auth::AuthContext;
use maidan_types::{AuditScope, NewAuditEvent, TaskFingerprint, ThreadId};

use crate::error::ApiError;
use crate::extract::ApiPath;
use crate::routes::{cap, ApiResult};
use crate::state::AppState;

/// A thread's task fingerprint, or `404` when it has none (no title or
/// description, or created before fingerprints). The linked task is named
/// only when the caller may read it. `workspace:read` + thread access.
pub async fn get_task_fingerprint(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<TaskFingerprint>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    let mut fingerprint = state
        .store
        .get_task_fingerprint(thread_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if let Some(original) = fingerprint.duplicate_of
        && maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, original)
            .await
            .is_err()
    {
        fingerprint.duplicate_of = None;
    }
    Ok(Json(fingerprint))
}

/// Clear a thread's duplicate link (C8b): a live duplicate is withheld from
/// claims, so this is how a task wrongly linked becomes claimable. `204`;
/// `404` when the thread has no fingerprint, and the same `404` when it is
/// linked to an original the caller cannot see, so a clear never confirms a
/// thread the caller may not read (#1580); a thread in another
/// workspace or a private channel likewise answers `404`, not `403`. Writes a
/// `thread.duplicate_cleared` audit event with the clear.
/// `thread:transition` + access to the thread and its original.
pub async fn clear_task_duplicate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    // A thread the caller cannot see answers as missing, never `403`.
    let scope = match maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await {
        Err(maidan_auth::AuthError::Forbidden(_)) => return Err(ApiError::NotFound),
        other => other?,
    };
    let fingerprint = state
        .store
        .get_task_fingerprint(thread_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if let Some(original) = fingerprint.duplicate_of
        && !maidan_auth::can_access_thread(state.store.as_ref(), &auth, original).await?
    {
        return Err(ApiError::NotFound);
    }
    let audit = NewAuditEvent {
        scope: AuditScope::Workspace(scope.workspace_id),
        actor_id: Some(auth.actor_id),
        action: "thread.duplicate_cleared".into(),
        target_kind: Some("thread".into()),
        target_id: Some(thread_id.0),
        metadata: serde_json::json!({
            "duplicate_of": fingerprint.duplicate_of.map(|t| t.0),
        }),
    };
    state.store.clear_task_duplicate(thread_id, audit).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Why an open thread is withheld as a live duplicate (#1681), or `None`;
/// see [`maidan_mcp::withheld::withheld_as_duplicate`].
pub(crate) async fn withheld_as_duplicate(
    state: &AppState,
    auth: &AuthContext,
    thread: &maidan_types::Thread,
) -> Result<Option<maidan_types::ThreadWithheld>, ApiError> {
    Ok(maidan_mcp::withheld::withheld_as_duplicate(state.store.as_ref(), auth, thread).await?)
}
