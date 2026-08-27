//! A2A v1.0 gRPC binding (§10).
//!
//! A tonic server exposing the A2A task read/cancel/list operations, as thin
//! adapters over the same `dispatch_*` handlers the JSON-RPC and REST bindings
//! use. The proto is compiled locally and the generated code vendored
//! (`generated.rs`) so no build-time `protoc` is needed in CI or the image.
//!
//! Config-gated: the server only starts when `MAIDAN_A2A_GRPC_ADDR` is set, so
//! default deployments, CI, and tests are unaffected. Streaming ops
//! (SendStreamingMessage, SubscribeToTask), SendMessage, push configs, and the
//! extended card are deferred to follow-up clusters.

#[allow(
    clippy::all,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    missing_docs
)]
pub mod generated;

use std::net::SocketAddr;

use maidan_auth::{resolve_bearer, AuthContext};
use tonic::{Request, Response, Status};

use crate::a2a_agent;
use crate::state::AppState;
use generated::a2a_service_server::{A2aService, A2aServiceServer};
use generated::{
    CancelTaskRequest, GetTaskRequest, ListTasksRequest, ListTasksResponse, Task, TaskStatus,
};

/// The gRPC A2A service, backed by the shared [`AppState`].
pub struct GrpcA2a {
    state: AppState,
}

/// Resolve the caller's [`AuthContext`] from the gRPC request metadata, mirroring
/// the HTTP bearer middleware (bypass when auth is disabled).
async fn auth_from_grpc<T>(state: &AppState, request: &Request<T>) -> Result<AuthContext, Status> {
    if state.auth_disabled {
        return Ok(AuthContext::bypass());
    }
    let secret = request
        .metadata()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(crate::auth::parse_bearer)
        .ok_or_else(|| Status::unauthenticated("missing bearer token"))?;
    resolve_bearer(state.store.as_ref(), secret)
        .await
        .map_err(|_| Status::unauthenticated("invalid token"))
}

/// Map an operation's `JsonRpcResponse` result into a gRPC value or `Status`.
/// (Maidan overloads `-32001` for auth failures → `permission_denied`, and
/// `-32602` for both invalid-params and not-found → `invalid_argument`.)
fn op_value(
    result: Result<maidan_a2a::JsonRpcResponse, maidan_a2a::JsonRpcResponse>,
) -> Result<serde_json::Value, Status> {
    let resp = match result {
        Ok(r) => r,
        Err(r) => r,
    };
    if let Some(value) = resp.result {
        return Ok(value);
    }
    let (code, message) = resp
        .error
        .map(|e| (e.code, e.message))
        .unwrap_or((-32603, "internal error".to_string()));
    Err(match code {
        -32001 => Status::permission_denied(message),
        -32602 => Status::invalid_argument(message),
        -32603 => Status::internal(message),
        _ => Status::invalid_argument(message),
    })
}

/// Build the proto [`Task`] from Maidan's task JSON.
fn task_from_json(v: &serde_json::Value) -> Task {
    Task {
        id: v
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        context_id: v
            .get("contextId")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        status: Some(TaskStatus {
            state: v
                .get("status")
                .and_then(|s| s.get("state"))
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
        }),
    }
}

#[tonic::async_trait]
impl A2aService for GrpcA2a {
    async fn get_task(&self, request: Request<GetTaskRequest>) -> Result<Response<Task>, Status> {
        let auth = auth_from_grpc(&self.state, &request).await?;
        let id = request.into_inner().id;
        let params = serde_json::json!({ "id": id });
        let value = op_value(
            a2a_agent::dispatch_get_task(&self.state, &auth, a2a_agent::rest_id(), params).await,
        )?;
        Ok(Response::new(task_from_json(&value)))
    }

    async fn cancel_task(
        &self,
        request: Request<CancelTaskRequest>,
    ) -> Result<Response<Task>, Status> {
        let auth = auth_from_grpc(&self.state, &request).await?;
        let id = request.into_inner().id;
        let params = serde_json::json!({ "id": id });
        let value = op_value(
            a2a_agent::dispatch_tasks_cancel(&self.state, &auth, a2a_agent::rest_id(), params)
                .await,
        )?;
        Ok(Response::new(task_from_json(&value)))
    }

    async fn list_tasks(
        &self,
        request: Request<ListTasksRequest>,
    ) -> Result<Response<ListTasksResponse>, Status> {
        let auth = auth_from_grpc(&self.state, &request).await?;
        let req = request.into_inner();
        let mut params = serde_json::Map::new();
        if !req.context_id.is_empty() {
            params.insert(
                "contextId".into(),
                serde_json::Value::String(req.context_id),
            );
        }
        if req.page_size > 0 {
            params.insert("pageSize".into(), serde_json::Value::from(req.page_size));
        }
        let value = op_value(
            a2a_agent::dispatch_list_tasks(&self.state, &auth, a2a_agent::rest_id(), params.into())
                .await,
        )?;
        let tasks = value
            .get("tasks")
            .and_then(|t| t.as_array())
            .map(|arr| arr.iter().map(task_from_json).collect())
            .unwrap_or_default();
        Ok(Response::new(ListTasksResponse {
            tasks,
            next_page_token: String::new(),
        }))
    }
}

/// Build the tonic service for the A2A gRPC binding.
pub fn service(state: AppState) -> A2aServiceServer<GrpcA2a> {
    A2aServiceServer::new(GrpcA2a { state })
}

/// Serve the A2A gRPC binding on `addr` until the process exits. Called from
/// `main.rs` only when `MAIDAN_A2A_GRPC_ADDR` is set.
pub async fn serve(state: AppState, addr: SocketAddr) -> Result<(), tonic::transport::Error> {
    tonic::transport::Server::builder()
        .add_service(service(state))
        .serve(addr)
        .await
}
