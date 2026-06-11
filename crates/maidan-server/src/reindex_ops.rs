//! Operator HTTP API to enqueue embedding reindex jobs (Cluster 87.0).
//!
//! Jobs are persisted in the store (Cluster 104.0.3), not held per-replica, so
//! a job started on one replica is visible from any replica and survives
//! restart.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use maidan_auth::{
    capability::{TOKEN_ADMIN, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_types::{NewAuditEvent, ReindexJob, ReindexJobStatus, WorkspaceId};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Deserialize, ToSchema)]
pub struct StartReindexEmbeddings {
    pub workspace_id: Option<Uuid>,
}

fn cap(auth: &AuthContext, capability: &str) -> ApiResult<()> {
    auth.require_capability(capability).map_err(Into::into)
}

fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

pub async fn start_reindex_embeddings(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<StartReindexEmbeddings>,
) -> ApiResult<(StatusCode, Json<ReindexJob>)> {
    let workspace_id = body.workspace_id.map(WorkspaceId);
    match workspace_id {
        Some(wid) => {
            cap(&auth, WORKSPACE_WRITE)?;
            ensure_workspace(&auth, wid)?;
            state.store.get_workspace(wid).await?;
        }
        None => cap(&auth, TOKEN_ADMIN)?,
    }

    let job_id = Uuid::new_v4();
    let model = state.embedding_provider.model_name().to_string();
    let started_at = Utc::now();
    let job = ReindexJob {
        job_id,
        status: ReindexJobStatus::Running,
        workspace_id: workspace_id.map(|w| w.0),
        embedding_model: model.clone(),
        processed: None,
        failed: None,
        error: None,
        started_at,
        finished_at: None,
    };
    // Persist the Running record before returning so a GET on any replica sees
    // the job immediately.
    state.store.upsert_reindex_job(job.clone()).await?;

    let search = state.search.clone();
    let provider = state.embedding_provider.clone();
    let store = state.store.clone();
    let actor_id = if auth.bypass {
        None
    } else {
        Some(auth.member_id)
    };
    let audit_workspace = workspace_id.map(|w| w.0);
    let mut job_clone = job.clone();
    tokio::spawn(async move {
        let result = search
            .reindex_embeddings(provider.as_ref(), workspace_id)
            .await;
        job_clone.finished_at = Some(Utc::now());
        match result {
            Ok(report) => {
                job_clone.status = ReindexJobStatus::Completed;
                job_clone.processed = Some(report.processed);
                job_clone.failed = Some(report.failed);
            }
            Err(err) => {
                job_clone.status = ReindexJobStatus::Failed;
                job_clone.error = Some(err.to_string());
            }
        }
        if let Err(err) = store.upsert_reindex_job(job_clone.clone()).await {
            tracing::error!(%job_id, error = %err, "failed to persist reindex job result");
        }
        let _ = store
            .append_audit(NewAuditEvent {
                actor_id,
                action: "embeddings.reindex".into(),
                target_kind: Some("reindex_job".into()),
                target_id: Some(job_id),
                metadata: serde_json::json!({
                    "workspace_id": audit_workspace,
                    "embedding_model": model,
                    "job": job_clone,
                }),
            })
            .await;
    });

    Ok((StatusCode::ACCEPTED, Json(job)))
}

pub async fn get_reindex_embeddings_job(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(job_id): Path<Uuid>,
) -> ApiResult<Json<ReindexJob>> {
    let Some(job) = state.store.get_reindex_job(job_id).await? else {
        cap(&auth, TOKEN_ADMIN)?;
        return Err(ApiError::NotFound);
    };
    match job.workspace_id.map(WorkspaceId) {
        Some(wid) => {
            cap(&auth, WORKSPACE_WRITE)?;
            ensure_workspace(&auth, wid)?;
        }
        None => cap(&auth, TOKEN_ADMIN)?,
    }
    Ok(Json(job))
}
