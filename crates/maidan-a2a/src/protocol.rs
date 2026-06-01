//! Subset of the [A2A protocol](https://a2a-protocol.org/v1.0.0/specification) v1.0 JSON-RPC surface.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const JSONRPC_VERSION: &str = "2.0";
pub const METHOD_SEND_MESSAGE: &str = "SendMessage";
pub const METHOD_SEND_STREAMING_MESSAGE: &str = "SendStreamingMessage";
pub const METHOD_GET_TASK: &str = "GetTask";
pub const METHOD_SET_PUSH_NOTIFICATION_CONFIG: &str = "tasks/pushNotificationConfig/set";
pub const METHOD_GET_PUSH_NOTIFICATION_CONFIG: &str = "tasks/pushNotificationConfig/get";

pub const TASK_STATE_WORKING: &str = "TASK_STATE_WORKING";
pub const TASK_STATE_COMPLETED: &str = "TASK_STATE_COMPLETED";
pub const TASK_STATE_FAILED: &str = "TASK_STATE_FAILED";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(i64),
    Str(String),
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
    #[serde(skip_serializing_if = "Option::is_none")]
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

    pub fn error(id: JsonRpcId, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPart {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aMessage {
    pub role: String,
    pub parts: Vec<TextPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageRequest {
    pub message: A2aMessage,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTaskRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<A2aMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageResponse {
    pub task: Task,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatusUpdateEvent {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub status: TaskStatus,
    #[serde(rename = "final", default, skip_serializing_if = "std::ops::Not::not")]
    pub is_final: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamResponseTask {
    pub task: Task,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamResponseStatusUpdate {
    pub status_update: TaskStatusUpdateEvent,
}

/// Maidan routing hints carried in A2A `metadata.maidan`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaidanA2aContext {
    pub thread_id: Uuid,
    pub author_id: Uuid,
}

pub fn message_text(message: &A2aMessage) -> Option<String> {
    let lines: Vec<&str> = message
        .parts
        .iter()
        .filter(|p| p.kind == "text")
        .map(|p| p.text.as_str())
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushNotificationConfig {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPushNotificationConfigRequest {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPushNotificationConfigResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<PushNotificationConfig>,
}

pub fn maidan_context_from_metadata(metadata: &Option<Value>) -> Result<MaidanA2aContext, String> {
    let root = metadata
        .as_ref()
        .and_then(|v| v.get("maidan"))
        .ok_or_else(|| "metadata.maidan with thread_id and author_id is required".to_string())?;
    serde_json::from_value(root.clone()).map_err(|e| format!("invalid metadata.maidan: {e}"))
}
