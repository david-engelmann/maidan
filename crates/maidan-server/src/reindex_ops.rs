//! Operator HTTP API to enqueue embedding reindex jobs (Cluster 87.0).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use maidan_auth::{
    capability::{TOKEN_ADMIN, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_types::{NewAuditEvent, WorkspaceId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReindexJobStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReindexJob {
    pub job_id: Uuid,
    pub status: ReindexJobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<Uuid>,
    pub embedding_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
pub struct ReindexJobRegistry {
    jobs: RwLock<HashMap<Uuid, ReindexJob>>,
}

impl ReindexJobRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            jobs: RwLock::new(HashMap::new()),
        })
    }

    fn insert(&self, job: ReindexJob) {
        if let Ok(mut jobs) = self.jobs.write() {
            jobs.insert(job.job_id, job);
        }
    }

    fn get(&self, job_id: Uuid) -> Option<ReindexJob> {
        self.jobs.read().ok()?.get(&job_id).cloned()
    }

    fn update(&self, job_id: Uuid, update: impl FnOnce(&mut ReindexJob)) {
        if let Ok(mut jobs) = self.jobs.write() {
            if let Some(job) = jobs.get_mut(&job_id) {
                update(job);
            }
        }
    }
}

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
    state.reindex_jobs.insert(job.clone());

    let search = state.search.clone();
    let provider = state.embedding_provider.clone();
    let registry = state.reindex_jobs.clone();
    let store = state.store.clone();
    let actor_id = if auth.bypass {
        None
    } else {
        Some(auth.member_id)
    };
    let audit_workspace = workspace_id.map(|w| w.0);
    tokio::spawn(async move {
        let result = search
            .reindex_embeddings(provider.as_ref(), workspace_id)
            .await;
        let finished_at = Utc::now();
        match result {
            Ok(report) => {
                registry.update(job_id, |job| {
                    job.status = ReindexJobStatus::Completed;
                    job.processed = Some(report.processed);
                    job.failed = Some(report.failed);
                    job.finished_at = Some(finished_at);
                });
            }
            Err(err) => {
                registry.update(job_id, |job| {
                    job.status = ReindexJobStatus::Failed;
                    job.error = Some(err.to_string());
                    job.finished_at = Some(finished_at);
                });
            }
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
                    "job": registry.get(job_id),
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
    let job = state
        .reindex_jobs
        .get(job_id)
        .ok_or(ApiError::NotFound)?;
    match job.workspace_id.map(WorkspaceId) {
        Some(wid) => {
            cap(&auth, WORKSPACE_WRITE)?;
            ensure_workspace(&auth, wid)?;
        }
        None => cap(&auth, TOKEN_ADMIN)?,
    }
    Ok(Json(job))
}
