//! Task attributes over REST (see [`maidan_types::task_attributes`]):
//! replace a thread's kind, engine and threat-model flag after intake.
//! Reading them back is the thread read's job; they ride on
//! `GET /threads/{id}`.

use axum::{extract::State, Extension, Json};
use maidan_auth::capability::THREAD_TRANSITION;
use maidan_auth::AuthContext;
use maidan_types::{TaskAttributes, ThreadId};

use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
use crate::routes::{cap, ApiResult};
use crate::state::AppState;

/// Replace a thread's task attributes whole. `thread:transition` + thread
/// access: the attributes are part of managing the task, as its footprint
/// and its result are.
pub async fn set_thread_attributes(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<TaskAttributes>,
) -> ApiResult<Json<TaskAttributes>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(
        state
            .store
            .set_thread_attributes(thread_id, body.validated().map_err(ApiError::BadRequest)?)
            .await?,
    ))
}
