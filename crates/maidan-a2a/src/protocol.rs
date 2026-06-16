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
pub const METHOD_SUBSCRIBE_TO_TASK: &str = "SubscribeToTask";
pub const METHOD_TASKS_RESUBSCRIBE: &str = "tasks/resubscribe";
pub const METHOD_TASKS_CANCEL: &str = "tasks/cancel";

pub const TASK_STATE_WORKING: &str = "TASK_STATE_WORKING";
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

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
        assert!(!is_terminal_task_state(TASK_STATE_WORKING));
        assert!(!is_terminal_task_state("TASK_STATE_UNKNOWN"));
        assert!(!is_terminal_task_state(""));
    }

    #[test]
    fn json_rpc_id_is_untagged_number_or_string() {
        assert_eq!(
            serde_json::to_value(JsonRpcId::Number(7)).unwrap(),
            json!(7)
        );
        assert_eq!(
            serde_json::to_value(JsonRpcId::Str("abc".into())).unwrap(),
            json!("abc")
        );
        // Deserializes back from each JSON scalar shape.
        assert!(matches!(
            serde_json::from_value::<JsonRpcId>(json!(9)).unwrap(),
            JsonRpcId::Number(9)
        ));
        assert!(matches!(
            serde_json::from_value::<JsonRpcId>(json!("x")).unwrap(),
            JsonRpcId::Str(_)
        ));
    }

    #[test]
    fn response_constructors_set_result_xor_error() {
        let ok = JsonRpcResponse::success(JsonRpcId::Number(1), json!({"x": 1}));
        assert!(ok.result.is_some() && ok.error.is_none());

        let err = JsonRpcResponse::error(JsonRpcId::Number(1), -32000, "boom");
        assert!(err.result.is_none());
        let e = err.error.expect("error");
        assert_eq!(e.code, -32000);
        assert_eq!(e.message, "boom");
    }

    #[test]
    fn message_round_trips_and_message_text_joins_text_parts() {
        let msg = A2aMessage {
            role: "agent".into(),
            parts: vec![
                TextPart {
                    kind: "text".into(),
                    text: "line one".into(),
                },
                TextPart {
                    kind: "image".into(),
                    text: "ignored".into(),
                },
                TextPart {
                    kind: "text".into(),
                    text: "line two".into(),
                },
            ],
            metadata: None,
        };
        let back: A2aMessage =
            serde_json::from_value(serde_json::to_value(&msg).unwrap()).expect("round trip");
        assert_eq!(back.parts.len(), 3);
        assert_eq!(message_text(&msg).as_deref(), Some("line one\nline two"));
    }

    #[test]
    fn message_text_is_none_without_text_parts() {
        let msg = A2aMessage {
            role: "agent".into(),
            parts: vec![TextPart {
                kind: "image".into(),
                text: "x".into(),
            }],
            metadata: None,
        };
        assert_eq!(message_text(&msg), None);
    }

    #[test]
    fn task_round_trips_through_camel_case_json() {
        let task = Task {
            id: "t1".into(),
            context_id: Some("c1".into()),
            status: TaskStatus {
                state: TASK_STATE_WORKING.into(),
                message: None,
            },
            metadata: None,
        };
        let v = serde_json::to_value(&task).unwrap();
        assert_eq!(v["contextId"], "c1", "context_id renders camelCase");
        let back: Task = serde_json::from_value(v).expect("round trip");
        assert_eq!(back.id, "t1");
        assert_eq!(back.status.state, TASK_STATE_WORKING);
    }

    #[test]
    fn context_from_metadata_requires_the_maidan_block() {
        let thread = uuid::Uuid::new_v4();
        let author = uuid::Uuid::new_v4();
        let ok = maidan_context_from_metadata(&Some(json!({
            "maidan": {"threadId": thread, "authorId": author}
        })))
        .expect("valid context");
        assert_eq!(ok.thread_id, thread);
        assert_eq!(ok.author_id, author);

        assert!(maidan_context_from_metadata(&None).is_err());
        assert!(maidan_context_from_metadata(&Some(json!({"other": 1}))).is_err());
        // Present but malformed maidan block.
        assert!(
            maidan_context_from_metadata(&Some(json!({"maidan": {"threadId": "nope"}}))).is_err()
        );
    }

    proptest! {
        /// Fuzz terminal-state classification: true iff the string is one of the
        /// four finished states.
        #[test]
        fn is_terminal_matches_the_finished_set(s in "[A-Z_]{0,32}") {
            let expected = matches!(
                s.as_str(),
                "TASK_STATE_COMPLETED"
                    | "TASK_STATE_FAILED"
                    | "TASK_STATE_CANCELED"
                    | "TASK_STATE_REJECTED"
            );
            prop_assert_eq!(is_terminal_task_state(&s), expected);
        }

        /// Fuzz message_text: the result is `Some` iff at least one part has
        /// kind "text", and it joins exactly those parts' text with newlines.
        #[test]
        fn message_text_joins_only_text_parts(
            parts in prop::collection::vec(("(text|image|file)", "[a-z ]{0,12}"), 0..6)
        ) {
            let message = A2aMessage {
                role: "agent".into(),
                parts: parts
                    .iter()
                    .map(|(kind, text)| TextPart { kind: kind.clone(), text: text.clone() })
                    .collect(),
                metadata: None,
            };
            let expected: Vec<&str> = parts
                .iter()
                .filter(|(k, _)| k == "text")
                .map(|(_, t)| t.as_str())
                .collect();
            let got = message_text(&message);
            if expected.is_empty() {
                prop_assert!(got.is_none());
            } else {
                let joined = expected.join("\n");
                prop_assert_eq!(got.as_deref(), Some(joined.as_str()));
            }
        }
    }
}
