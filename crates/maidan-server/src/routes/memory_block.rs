//! Attachable labeled memory-block management (Cluster 373.2, Wave 2 #21, H11).
//! A memory block is a Letta-shaped `{label, description, limit, read_only,
//! value}` workspace object a thread attaches to (a "room object"), letting a
//! parent watch a child's result block without a nested runtime. Blocks are
//! workspace content, so CRUD is gated on `workspace:read`/`workspace:write`;
//! attach/detach additionally require access to the target thread.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_types::*;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

pub async fn create_memory_block(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMemoryBlock>,
) -> ApiResult<(StatusCode, Json<MemoryBlock>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;

    let label = body.label.trim().to_string();
    if !is_valid_block_label(&label) {
        return Err(ApiError::BadRequest(
            "label must be non-empty, trimmed, and at most 128 chars".into(),
        ));
    }
    let block = state
        .store
        .create_memory_block(NewMemoryBlock {
            workspace_id,
            label,
            description: body.description,
            char_limit: body.char_limit,
            read_only: body.read_only,
            value: body.value.unwrap_or_default(),
            owner_id: auth.member_id,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(block)))
}

pub async fn list_memory_blocks(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<MemoryBlock>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_memory_blocks(workspace_id).await?))
}

/// Resolve a block under `:wid` and authorize the caller for that workspace.
/// `NotFound` if the block is missing or lives in another workspace (no
/// cross-tenant existence oracle).
async fn resolve_block(
    state: &AppState,
    auth: &AuthContext,
    workspace_id: WorkspaceId,
    block_id: MemoryBlockId,
) -> ApiResult<MemoryBlock> {
    ensure_workspace(auth, workspace_id)?;
    let block = state
        .store
        .get_memory_block(block_id)
        .await?
        .filter(|b| b.workspace_id == workspace_id)
        .ok_or(ApiError::NotFound)?;
    Ok(block)
}

pub async fn get_memory_block(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, block_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<Json<MemoryBlock>> {
    cap(&auth, WORKSPACE_READ)?;
    let block = resolve_block(
        &state,
        &auth,
        WorkspaceId(workspace_id),
        MemoryBlockId(block_id),
    )
    .await?;
    Ok(Json(block))
}

pub async fn set_memory_block_value(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, block_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    ApiJson(body): ApiJson<SetMemoryBlockValue>,
) -> ApiResult<Json<MemoryBlock>> {
    cap(&auth, WORKSPACE_WRITE)?;
    let block_id = MemoryBlockId(block_id);
    resolve_block(&state, &auth, WorkspaceId(workspace_id), block_id).await?;
    // A read-only block or an over-limit value surfaces as InvalidInput → 400.
    let updated = state
        .store
        .set_memory_block_value(block_id, &body.value)
        .await?;
    // A "go fetch" pointer so a parent watching the block reacts without polling
    // (Cluster 373.4). Best-effort — a bus hiccup never fails the write.
    super::publish(
        &state,
        Event::MemoryBlockUpdated {
            occurred_at: chrono::Utc::now(),
            workspace_id: updated.workspace_id,
            block_id: updated.id,
            label: updated.label.clone(),
            updated_by: auth.member_id,
        },
    )
    .await;
    Ok(Json(updated))
}

pub async fn delete_memory_block(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, block_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_WRITE)?;
    let block_id = MemoryBlockId(block_id);
    resolve_block(&state, &auth, WorkspaceId(workspace_id), block_id).await?;
    if state.store.delete_memory_block(block_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// Resolve a block for a thread-attachment operation: the block must exist, live
/// in the caller's workspace, and share the thread's workspace; the caller must
/// have access to the thread. Returns the block on success.
async fn authorize_attachment(
    state: &AppState,
    auth: &AuthContext,
    thread_id: ThreadId,
    block_id: MemoryBlockId,
) -> ApiResult<()> {
    let block = state
        .store
        .get_memory_block(block_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    ensure_workspace(auth, block.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), auth, thread_id).await?;
    let ctx = maidan_router::resolve_thread_context(state.store.as_ref(), thread_id).await?;
    if ctx.workspace_id != block.workspace_id {
        return Err(ApiError::BadRequest(
            "thread and block are in different workspaces".into(),
        ));
    }
    Ok(())
}

pub async fn attach_memory_block(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((thread_id, block_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_WRITE)?;
    let thread_id = ThreadId(thread_id);
    let block_id = MemoryBlockId(block_id);
    authorize_attachment(&state, &auth, thread_id, block_id).await?;
    state.store.attach_memory_block(thread_id, block_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn detach_memory_block(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((thread_id, block_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_WRITE)?;
    let thread_id = ThreadId(thread_id);
    let block_id = MemoryBlockId(block_id);
    authorize_attachment(&state, &auth, thread_id, block_id).await?;
    if state.store.detach_memory_block(thread_id, block_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn list_thread_memory_blocks(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(thread_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<MemoryBlock>>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(thread_id);
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(
        state.store.list_thread_memory_blocks(thread_id).await?,
    ))
}
