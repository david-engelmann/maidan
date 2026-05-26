//! MCP tools backed by [`maidan_store::Store`]. Each tool has a JSON
//! schema (input shape) and a dispatcher that decodes args, calls the
//! store, and returns a JSON result.

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::Bytes;
use maidan_artifacts::ArtifactStore;
use maidan_auth::capability::{
    ARTIFACT_UPLOAD, MESSAGE_POST, SEARCH_QUERY, WORKSPACE_READ, WORKSPACE_WRITE,
};
use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::McpError;

pub fn required_capability(name: &str) -> Result<&'static str, McpError> {
    match name {
        "list_channels" | "list_threads" | "list_messages" | "get_artifact_metadata" => {
            Ok(WORKSPACE_READ)
        }
        "post_message" => Ok(MESSAGE_POST),
        "record_mention" | "cast_vote" | "add_reference" => Ok(WORKSPACE_WRITE),
        "upload_artifact" => Ok(ARTIFACT_UPLOAD),
        "search_messages" => Ok(SEARCH_QUERY),
        other => Err(McpError::MethodNotFound(format!("tools/{other}"))),
    }
}

/// Catalog of every tool the MCP server exposes. The JSON-RPC client
/// receives this verbatim in the `tools/list` response.
pub fn catalog() -> Vec<Value> {
    vec![
        json!({
            "name": "list_channels",
            "description": "List channels in a workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"}
                },
                "required": ["workspace_id"]
            }
        }),
        json!({
            "name": "list_threads",
            "description": "List threads in a channel.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel_id": {"type": "string", "format": "uuid"}
                },
                "required": ["channel_id"]
            }
        }),
        json!({
            "name": "list_messages",
            "description": "List messages in a thread.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "limit": {"type": "integer", "default": 100}
                },
                "required": ["thread_id"]
            }
        }),
        json!({
            "name": "post_message",
            "description": "Post a message to a thread on behalf of a member.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "author_id": {"type": "string", "format": "uuid"},
                    "body": {"type": "string"},
                    "metadata": {"type": "object"}
                },
                "required": ["thread_id", "author_id", "body"]
            }
        }),
        json!({
            "name": "record_mention",
            "description": "Mark a member as mentioned in a message.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["message_id", "member_id"]
            }
        }),
        json!({
            "name": "cast_vote",
            "description": "Cast a vote on a message (e.g. approve, request-changes, emoji).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid"},
                    "kind": {"type": "string"}
                },
                "required": ["message_id", "member_id", "kind"]
            }
        }),
        json!({
            "name": "add_reference",
            "description": "Add a typed reference between two threads or messages.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "src_kind": {"type": "string", "enum": ["thread", "message"]},
                    "src_id": {"type": "string", "format": "uuid"},
                    "dst_kind": {"type": "string", "enum": ["thread", "message"]},
                    "dst_id": {"type": "string", "format": "uuid"},
                    "relation": {"type": "string"}
                },
                "required": ["src_kind", "src_id", "dst_kind", "dst_id", "relation"]
            }
        }),
        json!({
            "name": "upload_artifact",
            "description": "Store bytes in the artifact substrate and register metadata.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["screenshot", "recording", "transcript", "code_dump", "attachment"]
                    },
                    "content_base64": {"type": "string"},
                    "mime_type": {"type": "string"},
                    "uploaded_by": {"type": "string", "format": "uuid"}
                },
                "required": ["kind", "content_base64"]
            }
        }),
        json!({
            "name": "get_artifact_metadata",
            "description": "Fetch artifact metadata by sha256 hex digest.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sha256": {"type": "string", "minLength": 64, "maxLength": 64}
                },
                "required": ["sha256"]
            }
        }),
        json!({
            "name": "search_messages",
            "description": "Lexical full-text search over a workspace's messages. Returns ranked hits with highlighted snippets.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "query": {"type": "string", "minLength": 1},
                    "limit": {"type": "integer", "default": 25},
                    "author_id": {"type": "string", "format": "uuid"},
                    "channel_id": {"type": "string", "format": "uuid"},
                    "kind": {"type": "string", "enum": ["human", "agent"]}
                },
                "required": ["workspace_id", "query"]
            }
        }),
    ]
}

pub async fn dispatch(
    store: &Arc<dyn Store>,
    artifacts: &Arc<dyn ArtifactStore>,
    search: &Arc<dyn maidan_search::Search>,
    _auth: &AuthContext,
    name: &str,
    args: &Value,
) -> Result<Value, McpError> {
    match name {
        "list_channels" => list_channels(store, args).await,
        "list_threads" => list_threads(store, args).await,
        "list_messages" => list_messages(store, args).await,
        "post_message" => post_message(store, args).await,
        "record_mention" => record_mention(store, args).await,
        "cast_vote" => cast_vote(store, args).await,
        "add_reference" => add_reference(store, args).await,
        "upload_artifact" => upload_artifact(store, artifacts, args).await,
        "get_artifact_metadata" => get_artifact_metadata(store, args).await,
        "search_messages" => search_messages(search, args).await,
        other => Err(McpError::MethodNotFound(format!("tools/{other}"))),
    }
}

#[derive(Deserialize)]
struct SearchMessagesArgs {
    workspace_id: uuid::Uuid,
    query: String,
    #[serde(default = "default_search_limit")]
    limit: i64,
    author_id: Option<uuid::Uuid>,
    channel_id: Option<uuid::Uuid>,
    kind: Option<maidan_types::MemberKind>,
}

fn default_search_limit() -> i64 {
    25
}

#[derive(Deserialize)]
struct UploadArtifactArgs {
    kind: ArtifactKind,
    content_base64: String,
    mime_type: Option<String>,
    uploaded_by: Option<uuid::Uuid>,
}

async fn upload_artifact(
    store: &Arc<dyn Store>,
    artifacts: &Arc<dyn ArtifactStore>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: UploadArtifactArgs = serde_json::from_value(args.clone())?;
    let raw = STANDARD
        .decode(&a.content_base64)
        .map_err(|e| McpError::InvalidParams(format!("invalid base64: {e}")))?;
    let bytes = Bytes::from(raw);
    let sha = artifacts
        .put(bytes.clone())
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    let artifact = store
        .upsert_artifact(NewArtifact {
            sha256: sha.to_string(),
            size_bytes: bytes.len() as i64,
            mime_type: a.mime_type,
            kind: a.kind,
            uploaded_by: a.uploaded_by.map(MemberId),
        })
        .await?;
    Ok(content_json(&artifact))
}

#[derive(Deserialize)]
struct GetArtifactMetadataArgs {
    sha256: String,
}

async fn get_artifact_metadata(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: GetArtifactMetadataArgs = serde_json::from_value(args.clone())?;
    let artifact = store.get_artifact_by_sha(&a.sha256).await?;
    Ok(content_json(&artifact))
}

async fn search_messages(
    search: &Arc<dyn maidan_search::Search>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SearchMessagesArgs = serde_json::from_value(args.clone())?;
    let filters = maidan_search::SearchFilters {
        author_id: a.author_id.map(maidan_types::MemberId),
        channel_id: a.channel_id.map(maidan_types::ChannelId),
        author_kind: a.kind,
    };
    let hits = search
        .search_messages(WorkspaceId(a.workspace_id), &a.query, a.limit, &filters)
        .await?;
    Ok(content_json(&hits))
}

#[derive(Deserialize)]
struct ListChannelsArgs {
    workspace_id: uuid::Uuid,
}

async fn list_channels(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListChannelsArgs = serde_json::from_value(args.clone())?;
    let channels = store.list_channels(WorkspaceId(a.workspace_id)).await?;
    Ok(content_json(&channels))
}

#[derive(Deserialize)]
struct ListThreadsArgs {
    channel_id: uuid::Uuid,
}

async fn list_threads(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListThreadsArgs = serde_json::from_value(args.clone())?;
    let threads = store.list_threads(ChannelId(a.channel_id)).await?;
    Ok(content_json(&threads))
}

#[derive(Deserialize)]
struct ListMessagesArgs {
    thread_id: uuid::Uuid,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    100
}

async fn list_messages(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListMessagesArgs = serde_json::from_value(args.clone())?;
    let messages = store.list_messages(ThreadId(a.thread_id), a.limit).await?;
    Ok(content_json(&messages))
}

#[derive(Deserialize)]
struct PostMessageArgs {
    thread_id: uuid::Uuid,
    author_id: uuid::Uuid,
    body: String,
    #[serde(default)]
    metadata: Value,
}

async fn post_message(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: PostMessageArgs = serde_json::from_value(args.clone())?;
    let msg = store
        .post_message(NewMessage {
            thread_id: ThreadId(a.thread_id),
            author_id: MemberId(a.author_id),
            body: a.body,
            metadata: if a.metadata.is_null() {
                json!({})
            } else {
                a.metadata
            },
        })
        .await?;
    Ok(content_json(&msg))
}

#[derive(Deserialize)]
struct RecordMentionArgs {
    message_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

async fn record_mention(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: RecordMentionArgs = serde_json::from_value(args.clone())?;
    store
        .record_mention(MessageId(a.message_id), MemberId(a.member_id))
        .await?;
    Ok(content_json(&json!({"ok": true})))
}

#[derive(Deserialize)]
struct CastVoteArgs {
    message_id: uuid::Uuid,
    member_id: uuid::Uuid,
    kind: String,
}

async fn cast_vote(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: CastVoteArgs = serde_json::from_value(args.clone())?;
    store
        .cast_vote(NewVote {
            message_id: MessageId(a.message_id),
            member_id: MemberId(a.member_id),
            kind: a.kind,
        })
        .await?;
    Ok(content_json(&json!({"ok": true})))
}

#[derive(Deserialize)]
struct AddReferenceArgs {
    src_kind: RefSide,
    src_id: uuid::Uuid,
    dst_kind: RefSide,
    dst_id: uuid::Uuid,
    relation: String,
}

async fn add_reference(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: AddReferenceArgs = serde_json::from_value(args.clone())?;
    let r = store
        .add_reference(NewReference {
            src_kind: a.src_kind,
            src_id: a.src_id,
            dst_kind: a.dst_kind,
            dst_id: a.dst_id,
            relation: a.relation,
        })
        .await?;
    Ok(content_json(&r))
}

/// Wrap a JSON payload in MCP's `content[]` envelope. The MCP spec
/// requires tool results to be an array of content parts; for now we
/// always return a single `text` part with the JSON-stringified value.
fn content_json<T: serde::Serialize>(value: &T) -> Value {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    json!({
        "content": [
            { "type": "text", "text": body }
        ],
        "isError": false
    })
}
