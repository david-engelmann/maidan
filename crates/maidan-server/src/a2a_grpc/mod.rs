//! A2A v1.0 gRPC binding (§10).
//!
//! A tonic server for the official `lf.a2a.v1.A2AService` (`proto/a2a.proto`,
//! vendored unmodified from a2aproject/A2A v1.0.1). Every call is a thin
//! adapter over the operations the JSON-RPC and HTTP+JSON bindings use: the
//! request is converted to the shared operation type through its ProtoJSON
//! form (the JSON the other bindings speak) and the result converted back, so
//! the three bindings cannot drift in what they accept or return. Calls name
//! their protocol version in `a2a-version` metadata. The generated code is
//! vendored (`generated.rs`, `generated.serde.rs`; see
//! `scripts/gen-a2a-grpc.sh`) so no build-time `protoc` is needed.
//!
//! Config-gated: the server only starts when `MAIDAN_A2A_GRPC_ADDR` is set.
//! The `tenant` field is accepted and ignored: one endpoint serves one agent.

#[allow(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    clippy::unwrap_used,
    clippy::expect_used,
    missing_docs,
    unused_qualifications
)]
pub mod generated;

use std::net::SocketAddr;
use std::pin::Pin;

use futures::{Stream, StreamExt};
use maidan_auth::{resolve_bearer, AuthContext};
use serde::{de::DeserializeOwned, Serialize};
use tonic::{Request, Response, Status};

use crate::a2a_agent::{self, card, ops, push};
use crate::state::AppState;
use generated as pb;
use generated::a2a_service_server::{A2aService, A2aServiceServer};

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

/// `google.rpc.Status`, the payload of `grpc-status-details-bin`.
#[derive(Clone, PartialEq, prost::Message)]
struct RpcStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<pbjson_types::Any>,
}

/// `google.rpc.ErrorInfo`.
#[derive(Clone, PartialEq, prost::Message)]
struct ErrorInfo {
    #[prost(string, tag = "1")]
    reason: String,
    #[prost(string, tag = "2")]
    domain: String,
    #[prost(btree_map = "string, string", tag = "3")]
    metadata: std::collections::BTreeMap<String, String>,
}

/// An operation failure as the gRPC status its kind maps to, with the A2A
/// error's `google.rpc.ErrorInfo` in the status details (§10.6), the same
/// detail the JSON-RPC and REST bindings carry.
fn status(err: maidan_a2a::A2aError) -> Status {
    use prost::Message as _;
    use tonic::Code;
    let code = match err.kind.rpc_status() {
        "NOT_FOUND" => Code::NotFound,
        "FAILED_PRECONDITION" => Code::FailedPrecondition,
        "UNIMPLEMENTED" => Code::Unimplemented,
        "PERMISSION_DENIED" => Code::PermissionDenied,
        "INVALID_ARGUMENT" => Code::InvalidArgument,
        _ => Code::Internal,
    };
    let Some(reason) = err.kind.reason() else {
        return Status::new(code, err.message);
    };
    let info = ErrorInfo {
        reason: reason.to_string(),
        domain: maidan_a2a::A2A_ERROR_DOMAIN.to_string(),
        metadata: err.metadata,
    };
    let details = RpcStatus {
        code: code as i32,
        message: err.message.clone(),
        details: vec![pbjson_types::Any {
            type_url: maidan_a2a::ERROR_INFO_TYPE.to_string(),
            value: info.encode_to_vec().into(),
        }],
    };
    Status::with_details(code, err.message, details.encode_to_vec().into())
}

/// The caller's auth, after checking the protocol version it names.
async fn caller<T>(state: &AppState, request: &Request<T>) -> Result<AuthContext, Status> {
    a2a_agent::check_grpc_version(request.metadata()).map_err(status)?;
    auth_from_grpc(state, request).await
}

/// A generated request as the shared operation type, through ProtoJSON. A
/// message the operation type refuses (a part with no content, a message
/// with no role) is the caller's error.
fn from_proto<P: Serialize, T: DeserializeOwned>(message: &P) -> Result<T, Status> {
    serde_json::to_value(message)
        .and_then(serde_json::from_value)
        .map_err(|e| Status::invalid_argument(e.to_string()))
}

/// An operation result as the generated message, through ProtoJSON.
fn to_proto<T: Serialize, P: DeserializeOwned>(value: &T) -> Result<P, Status> {
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|e| Status::internal(format!("result does not fit a2a.proto: {e}")))
}

/// A server-streaming response.
type EventStream = Pin<Box<dyn Stream<Item = Result<pb::StreamResponse, Status>> + Send>>;

fn events<S>(stream: S) -> EventStream
where
    S: Stream<Item = maidan_a2a::StreamResponse> + Send + 'static,
{
    Box::pin(stream.map(|event| to_proto(&event)))
}

impl GrpcA2a {
    /// Run `method` as the caller: the gRPC server has no request layer, so
    /// this is where its writes take on the caller's attribution and a change
    /// that recorded nothing gets its record.
    async fn run<T>(
        &self,
        auth: &AuthContext,
        method: maidan_a2a::Method,
        call: impl std::future::Future<Output = Result<T, maidan_a2a::A2aError>>,
    ) -> Result<T, maidan_a2a::A2aError> {
        a2a_agent::recorded(&self.state, auth, "grpc", method, call).await
    }
}

#[tonic::async_trait]
impl A2aService for GrpcA2a {
    async fn send_message(
        &self,
        request: Request<pb::SendMessageRequest>,
    ) -> Result<Response<pb::SendMessageResponse>, Status> {
        let auth = caller(&self.state, &request).await?;
        let req = from_proto(request.get_ref())?;
        let task = self
            .run(
                &auth,
                maidan_a2a::Method::SendMessage,
                ops::send_message(&self.state, &auth, req),
            )
            .await
            .map_err(status)?;
        let reply = maidan_a2a::SendMessageResponse::Task(task);
        Ok(Response::new(to_proto(&reply)?))
    }

    type SendStreamingMessageStream = EventStream;

    async fn send_streaming_message(
        &self,
        request: Request<pb::SendMessageRequest>,
    ) -> Result<Response<Self::SendStreamingMessageStream>, Status> {
        let auth = caller(&self.state, &request).await?;
        let req = from_proto(request.get_ref())?;
        let sent = self
            .run(
                &auth,
                maidan_a2a::Method::SendStreamingMessage,
                ops::send_streaming_message(&self.state, &auth, req),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(events(futures::stream::iter(sent))))
    }

    async fn get_task(
        &self,
        request: Request<pb::GetTaskRequest>,
    ) -> Result<Response<pb::Task>, Status> {
        let auth = caller(&self.state, &request).await?;
        let req = from_proto(request.get_ref())?;
        let found = self
            .run(
                &auth,
                maidan_a2a::Method::GetTask,
                ops::get_task(&self.state, &auth, req),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(to_proto(&found)?))
    }

    async fn list_tasks(
        &self,
        request: Request<pb::ListTasksRequest>,
    ) -> Result<Response<pb::ListTasksResponse>, Status> {
        let auth = caller(&self.state, &request).await?;
        let req = from_proto(request.get_ref())?;
        let listed = self
            .run(
                &auth,
                maidan_a2a::Method::ListTasks,
                ops::list_tasks(&self.state, &auth, req),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(to_proto(&listed)?))
    }

    async fn cancel_task(
        &self,
        request: Request<pb::CancelTaskRequest>,
    ) -> Result<Response<pb::Task>, Status> {
        let auth = caller(&self.state, &request).await?;
        let req = from_proto(request.get_ref())?;
        let canceled = self
            .run(
                &auth,
                maidan_a2a::Method::CancelTask,
                ops::cancel_task(&self.state, &auth, req),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(to_proto(&canceled)?))
    }

    type SubscribeToTaskStream = EventStream;

    async fn subscribe_to_task(
        &self,
        request: Request<pb::SubscribeToTaskRequest>,
    ) -> Result<Response<Self::SubscribeToTaskStream>, Status> {
        let auth = caller(&self.state, &request).await?;
        let req = from_proto(request.get_ref())?;
        let stream = self
            .run(
                &auth,
                maidan_a2a::Method::SubscribeToTask,
                ops::subscribe(&self.state, &auth, req),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(events(stream)))
    }

    async fn create_task_push_notification_config(
        &self,
        request: Request<pb::TaskPushNotificationConfig>,
    ) -> Result<Response<pb::TaskPushNotificationConfig>, Status> {
        let auth = caller(&self.state, &request).await?;
        let config = from_proto(request.get_ref())?;
        let created = self
            .run(
                &auth,
                maidan_a2a::Method::CreatePushNotificationConfig,
                push::create(&self.state, &auth, config),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(to_proto(&created)?))
    }

    async fn get_task_push_notification_config(
        &self,
        request: Request<pb::GetTaskPushNotificationConfigRequest>,
    ) -> Result<Response<pb::TaskPushNotificationConfig>, Status> {
        let auth = caller(&self.state, &request).await?;
        let req = from_proto(request.get_ref())?;
        let config = self
            .run(
                &auth,
                maidan_a2a::Method::GetPushNotificationConfig,
                push::get(&self.state, &auth, req),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(to_proto(&config)?))
    }

    async fn list_task_push_notification_configs(
        &self,
        request: Request<pb::ListTaskPushNotificationConfigsRequest>,
    ) -> Result<Response<pb::ListTaskPushNotificationConfigsResponse>, Status> {
        let auth = caller(&self.state, &request).await?;
        let req = from_proto(request.get_ref())?;
        let listed = self
            .run(
                &auth,
                maidan_a2a::Method::ListPushNotificationConfigs,
                push::list(&self.state, &auth, req),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(to_proto(&listed)?))
    }

    async fn get_extended_agent_card(
        &self,
        request: Request<pb::GetExtendedAgentCardRequest>,
    ) -> Result<Response<pb::AgentCard>, Status> {
        let auth = caller(&self.state, &request).await?;
        let card = self
            .run(
                &auth,
                maidan_a2a::Method::GetExtendedAgentCard,
                std::future::ready(card::extended(&self.state, &auth)),
            )
            .await
            .map_err(status)?;
        Ok(Response::new(to_proto(&card)?))
    }

    async fn delete_task_push_notification_config(
        &self,
        request: Request<pb::DeleteTaskPushNotificationConfigRequest>,
    ) -> Result<Response<pbjson_types::Empty>, Status> {
        let auth = caller(&self.state, &request).await?;
        let req = from_proto(request.get_ref())?;
        self.run(
            &auth,
            maidan_a2a::Method::DeletePushNotificationConfig,
            push::delete(&self.state, &auth, req),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pbjson_types::Empty::default()))
    }
}

/// Build the tonic service for the A2A gRPC binding.
pub fn service(state: AppState) -> A2aServiceServer<GrpcA2a> {
    A2aServiceServer::new(GrpcA2a { state })
}

/// Serve the A2A gRPC binding on `addr` until the process exits. Called from
/// `main.rs` only when `MAIDAN_A2A_GRPC_ADDR` is set.
/// Acknowledges that the gRPC listener is plaintext on purpose, because TLS is
/// terminated by a proxy or load balancer in front of it.
pub const PLAINTEXT_ACK: &str = "MAIDAN_A2A_GRPC_PLAINTEXT";

/// Whether a plaintext gRPC listener on `addr` may start. The binding serves
/// bearer tokens and task content over HTTP/2 with no TLS of its own, so off
/// the loopback interface it needs the operator to say TLS is handled in front.
pub fn check_plaintext_bind(addr: SocketAddr, acknowledged: bool) -> Result<(), String> {
    if addr.ip().is_loopback() || acknowledged {
        return Ok(());
    }
    Err(format!(
        "the A2A gRPC listener on {addr} is plaintext and not on loopback; terminate TLS \
         in front of it and set {PLAINTEXT_ACK}=1, or bind it to 127.0.0.1"
    ))
}

pub async fn serve(state: AppState, addr: SocketAddr) -> Result<(), tonic::transport::Error> {
    tonic::transport::Server::builder()
        .layer(crate::trace_context::GrpcTraceLayer)
        .add_service(service(state))
        .serve(addr)
        .await
}

#[cfg(test)]
mod plaintext_tests {
    use super::check_plaintext_bind;

    #[test]
    fn a_plaintext_listener_off_loopback_needs_the_acknowledgement() {
        let public = "0.0.0.0:50051".parse().unwrap();
        assert!(check_plaintext_bind(public, false).is_err());
        assert!(check_plaintext_bind(public, true).is_ok());
        assert!(check_plaintext_bind("127.0.0.1:50051".parse().unwrap(), false).is_ok());
        assert!(check_plaintext_bind("[::1]:50051".parse().unwrap(), false).is_ok());
    }
}
