//! Reference handlers: create and list cross-entity references.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_types::*;

use super::{cap, publish, ApiResult};
use crate::dto::*;
use crate::error::ApiJson;
use crate::state::AppState;

pub async fn create_reference(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiJson(body): ApiJson<CreateReference>,
) -> ApiResult<(StatusCode, Json<Reference>)> {
    cap(&auth, WORKSPACE_WRITE)?;
    let r = state
        .store
        .add_reference(NewReference {
            src_kind: body.src_kind,
            src_id: body.src_id,
            dst_kind: body.dst_kind,
            dst_id: body.dst_id,
            relation: body.relation,
        })
        .await?;
    publish(
        &state,
        Event::ReferenceAdded {
            occurred_at: Utc::now(),
            reference: r.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(r)))
}

pub async fn list_references(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<ListReferencesQuery>,
) -> ApiResult<Json<Vec<Reference>>> {
    cap(&auth, WORKSPACE_READ)?;
    Ok(Json(
        state
            .store
            .list_references_from(q.src_kind, q.src_id)
            .await?,
    ))
}
