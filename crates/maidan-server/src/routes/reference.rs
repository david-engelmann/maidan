//! Reference handlers: create and list cross-entity references.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_types::*;

use super::{cap, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

/// Ensure the caller may access a referenced entity — and, via the `ensure_*`
/// helpers, that it belongs to their workspace (Cluster 165; this path had no
/// workspace/access check before).
async fn ensure_ref_access(
    store: &dyn maidan_store::Store,
    auth: &AuthContext,
    kind: RefSide,
    id: uuid::Uuid,
) -> Result<(), ApiError> {
    match kind {
        RefSide::Thread => maidan_auth::ensure_thread_access(store, auth, ThreadId(id)).await?,
        RefSide::Message => maidan_auth::ensure_message_access(store, auth, MessageId(id)).await?,
    }
    Ok(())
}

pub async fn create_reference(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiJson(body): ApiJson<CreateReference>,
) -> ApiResult<(StatusCode, Json<Reference>)> {
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_ref_access(state.store.as_ref(), &auth, body.src_kind, body.src_id).await?;
    ensure_ref_access(state.store.as_ref(), &auth, body.dst_kind, body.dst_id).await?;
    let (r, stored) = state
        .store
        .add_reference_with_event(NewReference {
            src_kind: body.src_kind,
            src_id: body.src_id,
            dst_kind: body.dst_kind,
            dst_id: body.dst_id,
            relation: body.relation,
        })
        .await?;
    super::publish_stored(&state, stored).await;
    Ok((StatusCode::CREATED, Json(r)))
}

/// List references by source (forward) OR target (reverse), optionally filtered by
/// relation (Cluster 320). Exactly one of the `src_*` / `dst_*` pairs is required;
/// access is gated on that anchor entity (the Cluster-165 model), symmetric to
/// `create_reference`.
pub async fn list_references(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<ListReferencesQuery>,
) -> ApiResult<Json<Vec<Reference>>> {
    cap(&auth, WORKSPACE_READ)?;
    let mut refs = match (q.src_kind, q.src_id, q.dst_kind, q.dst_id) {
        (Some(src_kind), Some(src_id), None, None) => {
            ensure_ref_access(state.store.as_ref(), &auth, src_kind, src_id).await?;
            state.store.list_references_from(src_kind, src_id).await?
        }
        (None, None, Some(dst_kind), Some(dst_id)) => {
            ensure_ref_access(state.store.as_ref(), &auth, dst_kind, dst_id).await?;
            state.store.list_references_to(dst_kind, dst_id).await?
        }
        _ => {
            return Err(ApiError::BadRequest(
                "provide exactly one of the src_kind+src_id or dst_kind+dst_id pair".into(),
            ))
        }
    };
    if let Some(relation) = q.relation {
        refs.retain(|r| r.relation == relation);
    }
    Ok(Json(refs))
}
