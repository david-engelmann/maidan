//! A2A v1.0 gRPC binding (§10).
//!
//! A tonic server exposing the A2A task read/cancel/list operations, as thin
//! adapters over the same operations the JSON-RPC and HTTP+JSON bindings use.
//! Calls name their protocol version in `a2a-version` metadata. The proto is
//! compiled locally and the generated code vendored (`generated.rs`) so no
//! build-time `protoc` is needed in CI or the image.
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

use crate::a2a_agent::{self, ops};
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

/// An operation failure as the gRPC status its kind maps to (§10.6).
fn status(err: maidan_a2a::A2aError) -> Status {
    use tonic::Code;
    let code = match err.kind.rpc_status() {
        "NOT_FOUND" => Code::NotFound,
        "FAILED_PRECONDITION" => Code::FailedPrecondition,
        "UNIMPLEMENTED" => Code::Unimplemented,
        "PERMISSION_DENIED" => Code::PermissionDenied,
        "INVALID_ARGUMENT" => Code::InvalidArgument,
        _ => Code::Internal,
    };
    Status::new(code, err.message)
}

/// The caller's auth, after checking the protocol version it names.
async fn caller<T>(state: &AppState, request: &Request<T>) -> Result<AuthContext, Status> {
    a2a_agent::check_grpc_version(request.metadata()).map_err(status)?;
    auth_from_grpc(state, request).await
}

fn task(task: maidan_a2a::Task) -> Task {
    Task {
        id: task.id,
        context_id: task.context_id.unwrap_or_default(),
        status: Some(TaskStatus {
            state: task.status.state,
        }),
    }
}

#[tonic::async_trait]
impl A2aService for GrpcA2a {
    async fn get_task(&self, request: Request<GetTaskRequest>) -> Result<Response<Task>, Status> {
        let auth = caller(&self.state, &request).await?;
        let req = maidan_a2a::GetTaskRequest {
            id: request.into_inner().id,
            history_length: None,
        };
        let found = ops::get_task(&self.state, &auth, req)
            .await
            .map_err(status)?;
        Ok(Response::new(task(found)))
    }

    async fn cancel_task(
        &self,
        request: Request<CancelTaskRequest>,
    ) -> Result<Response<Task>, Status> {
        let auth = caller(&self.state, &request).await?;
        let req = maidan_a2a::CancelTaskRequest {
            id: request.into_inner().id,
            metadata: None,
        };
        let canceled = ops::cancel_task(&self.state, &auth, req)
            .await
            .map_err(status)?;
        Ok(Response::new(task(canceled)))
    }

    async fn list_tasks(
        &self,
        request: Request<ListTasksRequest>,
    ) -> Result<Response<ListTasksResponse>, Status> {
        let auth = caller(&self.state, &request).await?;
        let req = request.into_inner();
        let req = maidan_a2a::ListTasksRequest {
            context_id: (!req.context_id.is_empty()).then_some(req.context_id),
            page_size: (req.page_size > 0).then_some(req.page_size),
            ..Default::default()
        };
        let listed = ops::list_tasks(&self.state, &auth, req)
            .await
            .map_err(status)?;
        Ok(Response::new(ListTasksResponse {
            tasks: listed.tasks.into_iter().map(task).collect(),
            next_page_token: listed.next_page_token,
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
