//! The [A2A protocol](https://a2a-protocol.org/v1.0.0/specification) v1.0 data
//! model in its ProtoJSON form (camelCase fields, SCREAMING_SNAKE enums), plus
//! the JSON-RPC 2.0 envelope the JSON-RPC binding wraps it in. Parsing also
//! accepts the proto field names (`task_id`), as ProtoJSON parsers must.

use std::collections::BTreeMap;

use maidan_types::ContentBlock;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const JSONRPC_VERSION: &str = "2.0";

/// The protocol version Maidan's interfaces speak (`Major.Minor`, §3.6).
pub const A2A_PROTOCOL_VERSION: &str = "1.0";
/// The service parameter a client names its protocol version with (§3.2.6),
/// sent as an HTTP header or a query parameter of the same name.
pub const A2A_VERSION_HEADER: &str = "A2A-Version";
/// Header carrying a push config's `token` on each notification.
pub const NOTIFICATION_TOKEN_HEADER: &str = "X-A2A-Notification-Token";

// A2A v1.0 JSON-RPC method strings are the canonical operation names from the
// spec's §5.3 Method Mapping Reference (identical to the gRPC method names).
pub const METHOD_SEND_MESSAGE: &str = "SendMessage";
pub const METHOD_SEND_STREAMING_MESSAGE: &str = "SendStreamingMessage";
pub const METHOD_GET_TASK: &str = "GetTask";
pub const METHOD_LIST_TASKS: &str = "ListTasks";
pub const METHOD_CREATE_PUSH_NOTIFICATION_CONFIG: &str = "CreateTaskPushNotificationConfig";
pub const METHOD_GET_PUSH_NOTIFICATION_CONFIG: &str = "GetTaskPushNotificationConfig";
pub const METHOD_LIST_PUSH_NOTIFICATION_CONFIGS: &str = "ListTaskPushNotificationConfigs";
pub const METHOD_DELETE_PUSH_NOTIFICATION_CONFIG: &str = "DeleteTaskPushNotificationConfig";
pub const METHOD_SUBSCRIBE_TO_TASK: &str = "SubscribeToTask";
pub const METHOD_CANCEL_TASK: &str = "CancelTask";
pub const METHOD_GET_EXTENDED_AGENT_CARD: &str = "GetExtendedAgentCard";

pub const TASK_STATE_SUBMITTED: &str = "TASK_STATE_SUBMITTED";
pub const TASK_STATE_WORKING: &str = "TASK_STATE_WORKING";
/// The task is paused waiting on human input: a held approval gate on the
/// task's context thread. Non-terminal: the run resumes when the gate is
/// answered.
pub const TASK_STATE_INPUT_REQUIRED: &str = "TASK_STATE_INPUT_REQUIRED";
pub const TASK_STATE_AUTH_REQUIRED: &str = "TASK_STATE_AUTH_REQUIRED";
pub const TASK_STATE_COMPLETED: &str = "TASK_STATE_COMPLETED";
pub const TASK_STATE_FAILED: &str = "TASK_STATE_FAILED";
pub const TASK_STATE_CANCELED: &str = "TASK_STATE_CANCELED";
pub const TASK_STATE_REJECTED: &str = "TASK_STATE_REJECTED";

pub fn is_terminal_task_state(state: &str) -> bool {
    matches!(
        state,
        TASK_STATE_COMPLETED | TASK_STATE_FAILED | TASK_STATE_CANCELED | TASK_STATE_REJECTED
    )
}

/// Normalize a caller-supplied task-state filter to the canonical
/// `TASK_STATE_*` wire form. Case-insensitive; accepts the kebab form
/// (`input-required`), the bare enum (`INPUT_REQUIRED`), or the full string.
/// `None` for an unrecognized state (the caller should reject the filter).
pub fn normalize_task_state(s: &str) -> Option<&'static str> {
    let norm = s.trim().to_ascii_uppercase().replace('-', "_");
    let norm = norm.strip_prefix("TASK_STATE_").unwrap_or(&norm);
    match norm {
        "SUBMITTED" => Some(TASK_STATE_SUBMITTED),
        "WORKING" => Some(TASK_STATE_WORKING),
        "INPUT_REQUIRED" => Some(TASK_STATE_INPUT_REQUIRED),
        "AUTH_REQUIRED" => Some(TASK_STATE_AUTH_REQUIRED),
        "COMPLETED" => Some(TASK_STATE_COMPLETED),
        "FAILED" => Some(TASK_STATE_FAILED),
        "CANCELED" | "CANCELLED" => Some(TASK_STATE_CANCELED),
        "REJECTED" => Some(TASK_STATE_REJECTED),
        _ => None,
    }
}

/// Whether a requested `A2A-Version` names a protocol version this server
/// speaks. Only `Major.Minor` counts (§3.6): `1.0` and `1.0.3` both match,
/// while an empty value means 0.3, which Maidan does not serve.
pub fn is_supported_version(requested: &str) -> bool {
    let mut parts = requested.trim().split('.');
    let (Some(major), Some(minor)) = (parts.next(), parts.next()) else {
        return false;
    };
    let patch_ok = parts.next().is_none_or(|p| p.parse::<u32>().is_ok()) && parts.next().is_none();
    let mut want = A2A_PROTOCOL_VERSION.split('.');
    patch_ok
        && major.parse::<u32>().ok() == want.next().and_then(|m| m.parse().ok())
        && minor.parse::<u32>().ok() == want.next().and_then(|m| m.parse().ok())
}

// ===== JSON-RPC 2.0 envelope =====

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(i64),
    Str(String),
    /// JSON-RPC answers a request whose id could not be read with `null`.
    Null,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    pub fn success(id: JsonRpcId, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: JsonRpcId, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

// ===== Data model (§4) =====

/// Who sent a message (§4.1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    #[serde(rename = "ROLE_USER")]
    User,
    #[serde(rename = "ROLE_AGENT")]
    Agent,
}

/// A part's content: exactly one of `text`, `raw` (base64 bytes), `url`, or
/// `data` (any JSON value), per the proto `oneof content`.
#[derive(Debug, Clone, PartialEq)]
pub enum PartContent {
    Text(String),
    Raw(String),
    Url(String),
    Data(Value),
}

/// A unit of message or artifact content (§4.1.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PartWire", into = "PartWire")]
pub struct Part {
    pub content: PartContent,
    pub metadata: Option<Value>,
    pub filename: Option<String>,
    #[serde(alias = "media_type")]
    pub media_type: Option<String>,
}

impl Part {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: PartContent::Text(text.into()),
            metadata: None,
            filename: None,
            media_type: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "media_type")]
    media_type: Option<String>,
}

impl TryFrom<PartWire> for Part {
    type Error = String;

    fn try_from(wire: PartWire) -> Result<Self, Self::Error> {
        let content = match (wire.text, wire.raw, wire.url, wire.data) {
            (Some(text), None, None, None) => PartContent::Text(text),
            (None, Some(raw), None, None) => PartContent::Raw(raw),
            (None, None, Some(url), None) => PartContent::Url(url),
            (None, None, None, Some(data)) => PartContent::Data(data),
            _ => return Err("a part must set exactly one of text, raw, url or data".into()),
        };
        Ok(Self {
            content,
            metadata: wire.metadata,
            filename: wire.filename,
            media_type: wire.media_type,
        })
    }
}

impl From<Part> for PartWire {
    fn from(part: Part) -> Self {
        let mut wire = PartWire {
            text: None,
            raw: None,
            url: None,
            data: None,
            metadata: part.metadata,
            filename: part.filename,
            media_type: part.media_type,
        };
        match part.content {
            PartContent::Text(t) => wire.text = Some(t),
            PartContent::Raw(r) => wire.raw = Some(r),
            PartContent::Url(u) => wire.url = Some(u),
            PartContent::Data(d) => wire.data = Some(d),
        }
        wire
    }
}

/// One turn of communication (§4.1.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    #[serde(alias = "message_id")]
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "context_id")]
    pub context_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "task_id")]
    pub task_id: Option<String>,
    pub role: Role,
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        alias = "reference_task_ids"
    )]
    pub reference_task_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    /// ISO 8601 UTC with millisecond precision (§5.6.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// A task output (§4.1.7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    #[serde(alias = "artifact_id")]
    pub artifact_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "context_id")]
    pub context_id: Option<String>,
    pub status: TaskStatus,
    /// Omitted unless the caller asked for artifacts (`includeArtifacts`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<Artifact>>,
    /// Omitted when `historyLength` is 0 (§3.2.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<Message>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatusUpdateEvent {
    #[serde(alias = "task_id")]
    pub task_id: String,
    #[serde(alias = "context_id")]
    pub context_id: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskArtifactUpdateEvent {
    #[serde(alias = "task_id")]
    pub task_id: String,
    #[serde(alias = "context_id")]
    pub context_id: String,
    pub artifact: Artifact,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub append: bool,
    #[serde(
        default,
        skip_serializing_if = "std::ops::Not::not",
        alias = "last_chunk"
    )]
    pub last_chunk: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// `SendMessage` result: a task or a direct reply (`{"task": …}` or
/// `{"message": …}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SendMessageResponse {
    Task(Task),
    Message(Message),
}

/// One streaming event, also the push notification payload (§3.2.3, §4.3.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamResponse {
    Task(Task),
    Message(Message),
    StatusUpdate(TaskStatusUpdateEvent),
    ArtifactUpdate(TaskArtifactUpdateEvent),
}

// ===== Push notifications (§4.3) =====

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationInfo {
    pub scheme: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<String>,
}

/// A per-task webhook (§4.3.1). `id` is server-generated when omitted. On the
/// REST binding `taskId` comes from the path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPushNotificationConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, alias = "task_id")]
    pub task_id: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication: Option<AuthenticationInfo>,
}

// ===== Operation requests and responses (§3.1, §3.2) =====

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageConfiguration {
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        alias = "accepted_output_modes"
    )]
    pub accepted_output_modes: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "task_push_notification_config"
    )]
    pub task_push_notification_config: Option<TaskPushNotificationConfig>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "history_length"
    )]
    pub history_length: Option<i32>,
    #[serde(
        default,
        skip_serializing_if = "std::ops::Not::not",
        alias = "return_immediately"
    )]
    pub return_immediately: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
    pub message: Message,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<SendMessageConfiguration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTaskRequest {
    pub id: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "history_length"
    )]
    pub history_length: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelTaskRequest {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeToTaskRequest {
    pub id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksRequest {
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "context_id")]
    pub context_id: Option<String>,
    /// A `TASK_STATE_*` value; the kebab and bare forms are accepted too
    /// (see [`normalize_task_state`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "page_size")]
    pub page_size: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "page_token")]
    pub page_token: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "history_length"
    )]
    pub history_length: Option<i32>,
    /// RFC 3339 instant; keeps tasks whose status changed strictly after it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "status_timestamp_after"
    )]
    pub status_timestamp_after: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "include_artifacts"
    )]
    pub include_artifacts: Option<bool>,
}

/// `ListTasks` result. `nextPageToken` is always present and empty on the
/// last page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksResponse {
    pub tasks: Vec<Task>,
    #[serde(alias = "next_page_token")]
    pub next_page_token: String,
    #[serde(alias = "page_size")]
    pub page_size: i32,
    #[serde(alias = "total_size")]
    pub total_size: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTaskPushNotificationConfigRequest {
    #[serde(alias = "task_id")]
    pub task_id: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteTaskPushNotificationConfigRequest {
    #[serde(alias = "task_id")]
    pub task_id: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTaskPushNotificationConfigsRequest {
    #[serde(alias = "task_id")]
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "page_size")]
    pub page_size: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "page_token")]
    pub page_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTaskPushNotificationConfigsResponse {
    pub configs: Vec<TaskPushNotificationConfig>,
    #[serde(alias = "next_page_token")]
    pub next_page_token: String,
}

// ===== Errors (§3.3.2, §5.4) =====

/// The domain of every A2A `google.rpc.ErrorInfo`.
pub const A2A_ERROR_DOMAIN: &str = "a2a-protocol.org";
pub const ERROR_INFO_TYPE: &str = "type.googleapis.com/google.rpc.ErrorInfo";

/// Every error an A2A operation can return: the JSON-RPC 2.0 standard codes,
/// the A2A-specific errors, and a permission failure (the spec leaves its
/// JSON-RPC code to the implementation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum A2aErrorKind {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
    TaskNotFound,
    TaskNotCancelable,
    PushNotificationNotSupported,
    UnsupportedOperation,
    ContentTypeNotSupported,
    InvalidAgentResponse,
    ExtendedAgentCardNotConfigured,
    ExtensionSupportRequired,
    VersionNotSupported,
    PermissionDenied,
}

impl A2aErrorKind {
    pub fn json_rpc_code(self) -> i32 {
        match self {
            Self::ParseError => -32700,
            Self::InvalidRequest => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::InternalError => -32603,
            Self::TaskNotFound => -32001,
            Self::TaskNotCancelable => -32002,
            Self::PushNotificationNotSupported => -32003,
            Self::UnsupportedOperation => -32004,
            Self::ContentTypeNotSupported => -32005,
            Self::InvalidAgentResponse => -32006,
            Self::ExtendedAgentCardNotConfigured => -32007,
            Self::ExtensionSupportRequired => -32008,
            Self::VersionNotSupported => -32009,
            // JSON-RPC's implementation-defined server-error range, outside
            // the A2A block (-32001..).
            Self::PermissionDenied => -32000,
        }
    }

    /// HTTP status on the REST binding.
    pub fn http_status(self) -> u16 {
        match self {
            Self::ParseError
            | Self::InvalidRequest
            | Self::InvalidParams
            | Self::PushNotificationNotSupported
            | Self::UnsupportedOperation
            | Self::ExtendedAgentCardNotConfigured
            | Self::ExtensionSupportRequired
            | Self::VersionNotSupported => 400,
            Self::PermissionDenied => 403,
            Self::MethodNotFound | Self::TaskNotFound => 404,
            Self::TaskNotCancelable => 409,
            Self::ContentTypeNotSupported => 415,
            Self::InternalError => 500,
            Self::InvalidAgentResponse => 502,
        }
    }

    /// `google.rpc.Code` name: the gRPC status, and AIP-193's `status` field.
    pub fn rpc_status(self) -> &'static str {
        match self {
            Self::ParseError
            | Self::InvalidRequest
            | Self::InvalidParams
            | Self::ContentTypeNotSupported => "INVALID_ARGUMENT",
            Self::MethodNotFound
            | Self::PushNotificationNotSupported
            | Self::UnsupportedOperation
            | Self::VersionNotSupported => "UNIMPLEMENTED",
            Self::TaskNotFound => "NOT_FOUND",
            Self::TaskNotCancelable
            | Self::ExtendedAgentCardNotConfigured
            | Self::ExtensionSupportRequired => "FAILED_PRECONDITION",
            Self::InternalError | Self::InvalidAgentResponse => "INTERNAL",
            Self::PermissionDenied => "PERMISSION_DENIED",
        }
    }

    /// The `ErrorInfo.reason` of an A2A-specific error; `None` for the
    /// JSON-RPC standard errors and permission failures.
    pub fn reason(self) -> Option<&'static str> {
        Some(match self {
            Self::TaskNotFound => "TASK_NOT_FOUND",
            Self::TaskNotCancelable => "TASK_NOT_CANCELABLE",
            Self::PushNotificationNotSupported => "PUSH_NOTIFICATION_NOT_SUPPORTED",
            Self::UnsupportedOperation => "UNSUPPORTED_OPERATION",
            Self::ContentTypeNotSupported => "CONTENT_TYPE_NOT_SUPPORTED",
            Self::InvalidAgentResponse => "INVALID_AGENT_RESPONSE",
            Self::ExtendedAgentCardNotConfigured => "EXTENDED_AGENT_CARD_NOT_CONFIGURED",
            Self::ExtensionSupportRequired => "EXTENSION_SUPPORT_REQUIRED",
            Self::VersionNotSupported => "VERSION_NOT_SUPPORTED",
            Self::ParseError
            | Self::InvalidRequest
            | Self::MethodNotFound
            | Self::InvalidParams
            | Self::InternalError
            | Self::PermissionDenied => return None,
        })
    }

    /// The kind a JSON-RPC error code names, if any.
    pub fn from_json_rpc_code(code: i32) -> Option<Self> {
        ALL_ERROR_KINDS
            .iter()
            .copied()
            .find(|k| k.json_rpc_code() == code)
    }
}

const ALL_ERROR_KINDS: [A2aErrorKind; 15] = [
    A2aErrorKind::ParseError,
    A2aErrorKind::InvalidRequest,
    A2aErrorKind::MethodNotFound,
    A2aErrorKind::InvalidParams,
    A2aErrorKind::InternalError,
    A2aErrorKind::TaskNotFound,
    A2aErrorKind::TaskNotCancelable,
    A2aErrorKind::PushNotificationNotSupported,
    A2aErrorKind::UnsupportedOperation,
    A2aErrorKind::ContentTypeNotSupported,
    A2aErrorKind::InvalidAgentResponse,
    A2aErrorKind::ExtendedAgentCardNotConfigured,
    A2aErrorKind::ExtensionSupportRequired,
    A2aErrorKind::VersionNotSupported,
    A2aErrorKind::PermissionDenied,
];

/// An operation failure, rendered per binding: a JSON-RPC error object, an
/// AIP-193 body with the matching HTTP status, or a gRPC status.
#[derive(Debug, Clone, PartialEq)]
pub struct A2aError {
    pub kind: A2aErrorKind,
    pub message: String,
    /// `ErrorInfo.metadata` context (e.g. `taskId`).
    pub metadata: BTreeMap<String, String>,
}

impl A2aError {
    pub fn new(kind: A2aErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            metadata: BTreeMap::new(),
        }
    }

    pub fn with(mut self, key: &str, value: impl Into<String>) -> Self {
        self.metadata.insert(key.to_string(), value.into());
        self
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(A2aErrorKind::InvalidParams, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(A2aErrorKind::InternalError, message)
    }

    pub fn task_not_found(task_id: &str) -> Self {
        Self::new(A2aErrorKind::TaskNotFound, "task not found").with("taskId", task_id)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(A2aErrorKind::UnsupportedOperation, message)
    }

    /// The error's detail objects: one `google.rpc.ErrorInfo` for an
    /// A2A-specific error, none otherwise.
    pub fn details(&self) -> Vec<Value> {
        let Some(reason) = self.kind.reason() else {
            return Vec::new();
        };
        let mut info = serde_json::json!({
            "@type": ERROR_INFO_TYPE,
            "reason": reason,
            "domain": A2A_ERROR_DOMAIN,
        });
        if !self.metadata.is_empty() {
            info["metadata"] = serde_json::json!(self.metadata);
        }
        vec![info]
    }

    pub fn to_json_rpc(&self) -> JsonRpcError {
        let details = self.details();
        JsonRpcError {
            code: self.kind.json_rpc_code(),
            message: self.message.clone(),
            data: (!details.is_empty()).then_some(Value::Array(details)),
        }
    }

    /// The AIP-193 error body of the REST binding (§11.6).
    pub fn to_rest_body(&self) -> Value {
        let mut error = serde_json::json!({
            "code": self.kind.http_status(),
            "status": self.kind.rpc_status(),
            "message": self.message,
        });
        let details = self.details();
        if !details.is_empty() {
            error["details"] = Value::Array(details);
        }
        serde_json::json!({ "error": error })
    }
}

impl std::fmt::Display for A2aError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for A2aError {}

// ===== Maidan content mapping =====

/// The searchable text of a message: its text parts joined by newlines, or
/// `None` without any.
pub fn message_text(message: &Message) -> Option<String> {
    let lines: Vec<&str> = message
        .parts
        .iter()
        .filter_map(|p| match &p.content {
            PartContent::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Maidan content blocks for a message's parts: a text part becomes a
/// [`ContentBlock::Text`], a URL part a [`ContentBlock::ResourceLink`]. Raw and
/// data parts have no block form; callers reject them before this.
pub fn message_content(message: &Message) -> Vec<ContentBlock> {
    message
        .parts
        .iter()
        .filter_map(|p| match &p.content {
            PartContent::Text(text) => Some(ContentBlock::Text { text: text.clone() }),
            PartContent::Url(uri) => Some(ContentBlock::ResourceLink {
                uri: uri.clone(),
                mime_type: p.media_type.clone(),
                title: p.filename.clone(),
            }),
            PartContent::Raw(_) | PartContent::Data(_) => None,
        })
        .collect()
}

/// A2A parts for stored content blocks, the inverse of [`message_content`]:
/// `Text` and `ResourceLink` round-trip; `Code` renders as a fenced text part
/// and `ToolResult` as its text; `ToolUse` has no part form and is skipped.
pub fn message_parts_from_content(content: &[ContentBlock]) -> Vec<Part> {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(Part::text(text.clone())),
            ContentBlock::Code { language, code } => Some(Part::text(format!(
                "```{}\n{code}\n```",
                language.as_deref().unwrap_or("")
            ))),
            ContentBlock::ToolResult { content, .. } => Some(Part::text(content.clone())),
            ContentBlock::ResourceLink {
                uri,
                mime_type,
                title,
            } => Some(Part {
                content: PartContent::Url(uri.clone()),
                metadata: None,
                filename: title.clone(),
                media_type: mime_type.clone(),
            }),
            ContentBlock::ToolUse { .. } => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    fn user_message(parts: Vec<Part>) -> Message {
        Message {
            message_id: "m1".into(),
            context_id: None,
            task_id: None,
            role: Role::User,
            parts,
            metadata: None,
            extensions: vec![],
            reference_task_ids: vec![],
        }
    }

    #[test]
    fn terminal_states_are_exactly_the_four_finished_states() {
        for s in [
            TASK_STATE_COMPLETED,
            TASK_STATE_FAILED,
            TASK_STATE_CANCELED,
            TASK_STATE_REJECTED,
        ] {
            assert!(is_terminal_task_state(s), "{s} should be terminal");
        }
        for s in [
            TASK_STATE_SUBMITTED,
            TASK_STATE_WORKING,
            TASK_STATE_INPUT_REQUIRED,
            TASK_STATE_AUTH_REQUIRED,
            "",
        ] {
            assert!(!is_terminal_task_state(s), "{s} should not be terminal");
        }
    }

    #[test]
    fn normalize_task_state_accepts_kebab_enum_and_full_forms() {
        for s in [
            "input-required",
            "INPUT_REQUIRED",
            "TASK_STATE_INPUT_REQUIRED",
            "Input-Required",
        ] {
            assert_eq!(
                normalize_task_state(s),
                Some(TASK_STATE_INPUT_REQUIRED),
                "{s}"
            );
        }
        assert_eq!(normalize_task_state("cancelled"), Some(TASK_STATE_CANCELED));
        assert_eq!(
            normalize_task_state("submitted"),
            Some(TASK_STATE_SUBMITTED)
        );
        assert_eq!(normalize_task_state("not-a-state"), None);
        assert_eq!(normalize_task_state(""), None);
    }

    #[test]
    fn version_negotiation_matches_major_minor_only() {
        for ok in ["1.0", "1.0.0", "1.0.7", " 1.0 "] {
            assert!(is_supported_version(ok), "{ok}");
        }
        for bad in [
            "", "0.3", "99.0", "1", "1.1", "1.0.x", "1.0.0.0", "one.zero",
        ] {
            assert!(!is_supported_version(bad), "{bad}");
        }
    }

    #[test]
    fn part_is_a_oneof_over_its_content_fields() {
        let text: Part = serde_json::from_value(json!({"text": "hi"})).unwrap();
        assert_eq!(text, Part::text("hi"));
        let url: Part = serde_json::from_value(
            json!({"url": "https://x/y.png", "mediaType": "image/png", "filename": "y.png"}),
        )
        .unwrap();
        assert_eq!(url.content, PartContent::Url("https://x/y.png".into()));
        assert_eq!(url.media_type.as_deref(), Some("image/png"));
        let data: Part = serde_json::from_value(json!({"data": {"k": 1}})).unwrap();
        assert_eq!(data.content, PartContent::Data(json!({"k": 1})));
        assert!(serde_json::from_value::<Part>(json!({})).is_err());
        assert!(serde_json::from_value::<Part>(json!({"text": "a", "url": "b"})).is_err());
        // Serializes back to the flat ProtoJSON form.
        assert_eq!(
            serde_json::to_value(&url).unwrap(),
            json!({"url": "https://x/y.png", "mediaType": "image/png", "filename": "y.png"})
        );
    }

    #[test]
    fn message_uses_proto_json_names() {
        let v = json!({
            "messageId": "m1",
            "contextId": "c1",
            "role": "ROLE_USER",
            "parts": [{"text": "hello"}],
            "referenceTaskIds": ["t0"],
        });
        let msg: Message = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.reference_task_ids, vec!["t0".to_string()]);
        assert_eq!(serde_json::to_value(&msg).unwrap(), v);
        // The pre-1.0 lowercase role is not a role.
        let mut old = v;
        old["role"] = json!("user");
        assert!(serde_json::from_value::<Message>(old).is_err());
    }

    #[test]
    fn responses_are_oneof_wrappers() {
        let task = Task {
            id: "t1".into(),
            context_id: Some("c1".into()),
            status: TaskStatus {
                state: TASK_STATE_COMPLETED.into(),
                message: None,
                timestamp: Some("2026-09-28T15:00:00.000Z".into()),
            },
            artifacts: None,
            history: None,
            metadata: None,
        };
        let v = serde_json::to_value(SendMessageResponse::Task(task.clone())).unwrap();
        assert_eq!(v["task"]["contextId"], "c1");
        assert!(
            v["task"].get("history").is_none(),
            "absent history is omitted"
        );
        let event = StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
            task_id: "t1".into(),
            context_id: "c1".into(),
            status: task.status,
            metadata: None,
        });
        let v = serde_json::to_value(&event).unwrap();
        assert_eq!(v["statusUpdate"]["status"]["state"], TASK_STATE_COMPLETED);
        assert!(v["statusUpdate"].get("final").is_none());
        assert_eq!(serde_json::from_value::<StreamResponse>(v).unwrap(), event);
    }

    #[test]
    fn a2a_errors_carry_error_info_and_standard_errors_do_not() {
        let e = A2aError::task_not_found("t9");
        let rpc = e.to_json_rpc();
        assert_eq!(rpc.code, -32001);
        let data = rpc.data.expect("ErrorInfo");
        assert_eq!(data[0]["@type"], ERROR_INFO_TYPE);
        assert_eq!(data[0]["reason"], "TASK_NOT_FOUND");
        assert_eq!(data[0]["domain"], A2A_ERROR_DOMAIN);
        assert_eq!(data[0]["metadata"]["taskId"], "t9");
        let rest = e.to_rest_body();
        assert_eq!(rest["error"]["code"], 404);
        assert_eq!(rest["error"]["status"], "NOT_FOUND");
        assert_eq!(rest["error"]["details"][0]["reason"], "TASK_NOT_FOUND");

        let bad = A2aError::invalid_params("nope");
        assert!(bad.to_json_rpc().data.is_none());
        assert!(bad.to_rest_body()["error"].get("details").is_none());
    }

    #[test]
    fn error_codes_follow_the_spec_mapping() {
        use A2aErrorKind::*;
        let table = [
            (TaskNotFound, -32001, 404),
            (TaskNotCancelable, -32002, 409),
            (PushNotificationNotSupported, -32003, 400),
            (UnsupportedOperation, -32004, 400),
            (ContentTypeNotSupported, -32005, 415),
            (InvalidAgentResponse, -32006, 502),
            (ExtendedAgentCardNotConfigured, -32007, 400),
            (ExtensionSupportRequired, -32008, 400),
            (VersionNotSupported, -32009, 400),
        ];
        for (kind, code, http) in table {
            assert_eq!(kind.json_rpc_code(), code, "{kind:?}");
            assert_eq!(kind.http_status(), http, "{kind:?}");
            assert!(kind.reason().is_some(), "{kind:?}");
            assert_eq!(A2aErrorKind::from_json_rpc_code(code), Some(kind));
        }
        // Every code is distinct, so a code names one kind.
        let mut codes: Vec<i32> = ALL_ERROR_KINDS.iter().map(|k| k.json_rpc_code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), ALL_ERROR_KINDS.len());
    }

    #[test]
    fn content_round_trips_text_and_links() {
        let msg = user_message(vec![
            Part::text("one"),
            Part {
                content: PartContent::Url("https://x/doc".into()),
                metadata: None,
                filename: Some("Doc".into()),
                media_type: Some("text/html".into()),
            },
        ]);
        let blocks = message_content(&msg);
        assert_eq!(blocks.len(), 2);
        assert_eq!(message_parts_from_content(&blocks), msg.parts);
        assert_eq!(message_text(&msg).as_deref(), Some("one"));
    }

    #[test]
    fn stored_blocks_project_to_parts() {
        let parts = message_parts_from_content(&[
            ContentBlock::Code {
                language: Some("rust".into()),
                code: "fn a() {}".into(),
            },
            ContentBlock::ToolUse {
                id: "u1".into(),
                name: "search".into(),
                input: json!({"q": "x"}),
            },
            ContentBlock::ToolResult {
                tool_use_id: "u1".into(),
                content: "hit".into(),
                is_error: false,
            },
        ]);
        assert_eq!(
            parts,
            vec![Part::text("```rust\nfn a() {}\n```"), Part::text("hit")],
            "ToolUse has no part form"
        );
    }

    proptest! {
        /// message_text is `Some` iff a text part exists and joins exactly
        /// the text parts, in order.
        #[test]
        fn message_text_joins_only_text_parts(
            parts in prop::collection::vec((any::<bool>(), "[a-z ]{0,12}"), 0..6)
        ) {
            let message = user_message(
                parts
                    .iter()
                    .map(|(is_text, s)| if *is_text {
                        Part::text(s.clone())
                    } else {
                        Part { content: PartContent::Url(s.clone()), metadata: None, filename: None, media_type: None }
                    })
                    .collect(),
            );
            let expected: Vec<&str> = parts.iter().filter(|(t, _)| *t).map(|(_, s)| s.as_str()).collect();
            let got = message_text(&message);
            if expected.is_empty() {
                prop_assert!(got.is_none());
            } else {
                let joined = expected.join("\n");
                prop_assert_eq!(got.as_deref(), Some(joined.as_str()));
            }
        }
    }

    #[test]
    fn proto_field_names_parse_like_camel_case() {
        let config: TaskPushNotificationConfig =
            serde_json::from_value(json!({ "task_id": "t1", "url": "https://h/p" })).unwrap();
        assert_eq!(config.task_id, "t1");
        let part: Part =
            serde_json::from_value(json!({ "text": "hi", "media_type": "text/plain" })).unwrap();
        assert_eq!(part.media_type.as_deref(), Some("text/plain"));
        // Output stays camelCase.
        assert!(serde_json::to_value(&config)
            .unwrap()
            .get("taskId")
            .is_some());
    }
}
