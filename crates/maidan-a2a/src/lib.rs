//! Maidan federation wire format plus Google A2A protocol v1.0 client types.

pub mod batch;
pub mod client;
pub mod envelope;
pub mod error;
pub mod outbound;
pub mod page_token;
pub mod peer;
pub mod protocol;
pub mod rpc;
pub mod tasks;

#[cfg(test)]
pub mod test_support;

pub use batch::{FederatedEventBatch, MAX_FEDERATION_BATCH_SIZE};
pub use client::A2aClient;
pub use envelope::FederationEnvelope;
pub use error::{A2aClientError, FederationError};
pub use outbound::Outbound;
pub use peer::{validate_base_url, validate_peer_name, NewPeer, Peer};
pub use protocol::{
    is_supported_version, is_terminal_task_state, message_content, message_parts_from_content,
    message_text, normalize_task_state, A2aError, A2aErrorKind, Artifact, AuthenticationInfo,
    CancelTaskRequest, DeleteTaskPushNotificationConfigRequest,
    GetTaskPushNotificationConfigRequest, GetTaskRequest, JsonRpcError, JsonRpcId, JsonRpcRequest,
    JsonRpcResponse, ListTaskPushNotificationConfigsRequest,
    ListTaskPushNotificationConfigsResponse, ListTasksRequest, ListTasksResponse, Message, Part,
    PartContent, Role, SendMessageConfiguration, SendMessageRequest, SendMessageResponse,
    StreamResponse, SubscribeToTaskRequest, Task, TaskArtifactUpdateEvent,
    TaskPushNotificationConfig, TaskStatus, TaskStatusUpdateEvent, A2A_ERROR_DOMAIN,
    A2A_PROTOCOL_VERSION, A2A_VERSION_HEADER, ERROR_INFO_TYPE, JSONRPC_VERSION, METHOD_CANCEL_TASK,
    METHOD_CREATE_PUSH_NOTIFICATION_CONFIG, METHOD_DELETE_PUSH_NOTIFICATION_CONFIG,
    METHOD_GET_EXTENDED_AGENT_CARD, METHOD_GET_PUSH_NOTIFICATION_CONFIG, METHOD_GET_TASK,
    METHOD_LIST_PUSH_NOTIFICATION_CONFIGS, METHOD_LIST_TASKS, METHOD_SEND_MESSAGE,
    METHOD_SEND_STREAMING_MESSAGE, METHOD_SUBSCRIBE_TO_TASK, NOTIFICATION_TOKEN_HEADER,
    TASK_STATE_AUTH_REQUIRED, TASK_STATE_CANCELED, TASK_STATE_COMPLETED, TASK_STATE_FAILED,
    TASK_STATE_INPUT_REQUIRED, TASK_STATE_REJECTED, TASK_STATE_SUBMITTED, TASK_STATE_WORKING,
};
pub use rpc::{parse_envelope, parse_operation, Envelope, Method, Operation};
