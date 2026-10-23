//! A task's fingerprint over REST (Context Economics C8; see
//! [`maidan_types::task_fingerprint`]): the hash of its brief and inputs,
//! and the open task with the same fingerprint it was linked to when
//! created.

use axum::{extract::State, Extension, Json};
use maidan_auth::capability::WORKSPACE_READ;
use maidan_auth::AuthContext;
use maidan_types::{TaskFingerprint, ThreadId};

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
