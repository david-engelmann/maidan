//! `GET /workspaces/:id/thread-lookup`: the thread a Slack message or
//! channel, or a GitHub pull request, refers to, so a session holding only
//! the external reference finds its task without listing threads.

use axum::{extract::State, Extension, Json};
use maidan_auth::capability::WORKSPACE_READ;
use maidan_auth::AuthContext;
use maidan_types::{Thread, WorkspaceId};

use crate::dto::ThreadLookupQuery;
use crate::error::ApiError;
use crate::extract::{ApiPath, ApiQuery};
use crate::routes::{cap, ensure_workspace, ApiResult};
use crate::state::AppState;

/// `workspace:read`. A thread the caller may not read (a private channel it
/// is not in, a DM it is not part of) answers 404, as no thread does, so a
/// lookup cannot confirm one exists.
pub async fn find_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(query): ApiQuery<ThreadLookupQuery>,
) -> ApiResult<Json<Thread>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    let thread = maidan_auth::find_readable_thread(
        state.store.as_ref(),
        &auth,
        workspace_id,
        &maidan_types::ThreadLookup {
            slack_channel: query.slack_channel,
            slack_ts: query.slack_ts,
            repo: query.repo,
            pr_number: query.pr_number,
        },
    )
    .await?;
    thread.map(Json).ok_or(ApiError::NotFound)
}
