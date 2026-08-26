//! Subset of the [A2A protocol](https://a2a-protocol.org/v1.0.0/specification) v1.0 JSON-RPC surface.

use maidan_types::ContentBlock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const JSONRPC_VERSION: &str = "2.0";
// A2A v1.0 JSON-RPC method strings are the canonical operation names from the
// spec's §5.3 Method Mapping Reference (identical to the gRPC method names),
// not the older `message/send`-style paths and not an `a2a.`-prefixed form.
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

/// `ListTasks` request. Subset of the spec's `ListTasksRequest`: optional
/// `contextId` filter and `pageSize` (default 50, min 1). The `status` filter and
/// opaque page tokens are not yet implemented (single-page).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<i32>,
}

/// `ListTasks` response. `nextPageToken` is always empty until pagination lands.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksResponse {
    pub tasks: Vec<Task>,
    pub next_page_token: String,
    pub page_size: i32,
    pub total_size: i32,
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

/// Structured content blocks from the message's text parts (Cluster 194): one
/// [`ContentBlock::Text`] per text part, so an A2A message carries the same
/// structured `content` as REST/MCP posts (Cluster 173) instead of dropping it.
/// `None` when there are no text parts (mirrors [`message_text`]); `body` stays
/// the joined searchable projection.
pub fn message_content(message: &A2aMessage) -> Option<Vec<ContentBlock>> {
    let blocks: Vec<ContentBlock> = message
        .parts
        .iter()
        .filter(|p| p.kind == "text")
        .map(|p| ContentBlock::Text {
            text: p.text.clone(),
        })
        .collect();
    (!blocks.is_empty()).then_some(blocks)
}

/// Render structured content blocks back to A2A text parts (Cluster 267) — the
/// egress inverse of [`message_content`]. A2A parts are text-only, so each block
/// projects to its text form, mirroring `maidan_types::derive_body`'s per-block
/// rendering: `Text` → its text, `Code` → a fenced block, `ToolResult` → its
/// content, `ResourceLink` → its title or URI. A `ToolUse` block has no text
/// projection and is skipped. The common case (Text blocks from an A2A-ingested
/// message) round-trips faithfully back to the original parts.
pub fn message_parts_from_content(content: &[ContentBlock]) -> Vec<TextPart> {
    content
        .iter()
        .filter_map(content_block_text)
        .map(|text| TextPart {
            kind: "text".to_string(),
            text,
        })
        .collect()
}

fn content_block_text(block: &ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text { text } => Some(text.clone()),
        ContentBlock::Code { language, code } => Some(format!(
            "```{}\n{code}\n```",
            language.as_deref().unwrap_or("")
        )),
        ContentBlock::ToolResult { content, .. } => Some(content.clone()),
        ContentBlock::ResourceLink { uri, title, .. } => {
            Some(title.clone().unwrap_or_else(|| uri.clone()))
        }
        ContentBlock::ToolUse { .. } => None,
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

/// A2A v1.0 per-task push notification config (spec `TaskPushNotificationConfig`).
/// `id` is the stable config id (server-generated on create if the client omits it).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPushNotificationConfig {
    #[serde(rename = "id", default, skip_serializing_if = "Option::is_none")]
    pub config_id: Option<String>,
    pub task_id: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTaskPushNotificationConfigRequest {
    pub task_id: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteTaskPushNotificationConfigRequest {
    pub task_id: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTaskPushNotificationConfigsRequest {
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTaskPushNotificationConfigsResponse {
    pub configs: Vec<TaskPushNotificationConfig>,
    pub next_page_token: String,
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
    fn message_content_maps_text_parts_to_blocks() {
        let msg = A2aMessage {
            role: "agent".into(),
            parts: vec![
                TextPart {
                    kind: "text".into(),
                    text: "first".into(),
                },
                TextPart {
                    kind: "image".into(),
                    text: "ignored".into(),
                },
                TextPart {
                    kind: "text".into(),
                    text: "second".into(),
                },
            ],
            metadata: None,
        };
        let content = message_content(&msg).expect("some content");
        assert_eq!(content.len(), 2, "only the two text parts become blocks");
        assert_eq!(
            content,
            vec![
                ContentBlock::Text {
                    text: "first".into()
                },
                ContentBlock::Text {
                    text: "second".into()
                },
            ]
        );
        // No text parts → None, mirroring message_text.
        let no_text = A2aMessage {
            role: "agent".into(),
            parts: vec![TextPart {
                kind: "image".into(),
                text: "x".into(),
            }],
            metadata: None,
        };
        assert_eq!(message_content(&no_text), None);
    }

    #[test]
    fn message_parts_from_content_round_trips_text_and_projects_other_blocks() {
        // Text blocks (the A2A-ingested case) round-trip faithfully back to parts.
        let blocks = message_content(&A2aMessage {
            role: "agent".into(),
            parts: vec![
                TextPart {
                    kind: "text".into(),
                    text: "one".into(),
                },
                TextPart {
                    kind: "text".into(),
                    text: "two".into(),
                },
            ],
            metadata: None,
        })
        .unwrap();
        let parts = message_parts_from_content(&blocks);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].kind, "text");
        assert_eq!(parts[0].text, "one");
        assert_eq!(parts[1].text, "two");

        // Non-text blocks project to their text form; ToolUse (no projection) is skipped.
        let mixed = vec![
            ContentBlock::Code {
                language: Some("rust".into()),
                code: "fn a() {}".into(),
            },
            ContentBlock::ToolUse {
                id: "u1".into(),
                name: "search".into(),
                input: serde_json::json!({"q": "x"}),
            },
            ContentBlock::ToolResult {
                tool_use_id: "u1".into(),
                content: "hit".into(),
                is_error: false,
            },
            ContentBlock::ResourceLink {
                uri: "maidan://a/1".into(),
                mime_type: None,
                title: Some("Doc".into()),
            },
        ];
        let parts = message_parts_from_content(&mixed);
        assert_eq!(
            parts.len(),
            3,
            "ToolUse has no text projection and is skipped"
        );
        assert_eq!(parts[0].text, "```rust\nfn a() {}\n```");
        assert_eq!(parts[1].text, "hit");
        assert_eq!(parts[2].text, "Doc");
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
