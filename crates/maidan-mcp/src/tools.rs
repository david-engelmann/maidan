//! MCP tools backed by [`maidan_store::Store`]. Each tool has a JSON
//! schema (input shape) and a dispatcher that decodes args, calls the
//! store, and returns a JSON result.

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::Bytes;
use maidan_artifacts::{ArtifactStore, CompletedPart, MultipartUpload, S3Store};
use maidan_auth::capability::{
    ARTIFACT_UPLOAD, MESSAGE_POST, SEARCH_QUERY, WORKSPACE_READ, WORKSPACE_WRITE,
};
use maidan_auth::{encrypt_peer_secret, AuthContext, TokenSecret};
use maidan_router::route_mentions_for_message;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::McpError;

pub fn required_capability(name: &str) -> Result<&'static str, McpError> {
    match name {
        "list_channels"
        | "list_threads"
        | "list_messages"
        | "list_dm_conversations"
        | "list_reactions"
        | "list_pins"
        | "get_artifact_metadata" => Ok(WORKSPACE_READ),
        "open_dm_conversation" | "post_dm_message" | "post_message" | "edit_message" => {
            Ok(MESSAGE_POST)
        }
        "record_mention" | "cast_vote" | "add_reaction" | "remove_reaction" | "pin_message"
        | "unpin_message" | "add_reference" => Ok(WORKSPACE_WRITE),
        "upload_artifact"
        | "begin_artifact_multipart"
        | "upload_artifact_multipart_part"
        | "complete_artifact_multipart"
        | "abort_artifact_multipart" => Ok(ARTIFACT_UPLOAD),
        "search_messages" => Ok(SEARCH_QUERY),
        "register_slash_command" => Ok(WORKSPACE_WRITE),
        "list_slash_commands" => Ok(WORKSPACE_READ),
        other => Err(McpError::MethodNotFound(format!("tools/{other}"))),
    }
}

/// Catalog of every tool the MCP server exposes. The JSON-RPC client
/// receives this verbatim in the `tools/list` response.
pub fn catalog() -> Vec<Value> {
    vec![
        json!({
            "name": "open_dm_conversation",
            "description": "Open or fetch a 1:1 DM conversation between two workspace members.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid"},
                    "other_member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["workspace_id", "member_id", "other_member_id"]
            }
        }),
        json!({
            "name": "list_dm_conversations",
            "description": "List DM conversations for a member in a workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["workspace_id", "member_id"]
            }
        }),
        json!({
            "name": "post_dm_message",
            "description": "Post a message in a DM conversation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "dm_conversation_id": {"type": "string", "format": "uuid"},
                    "author_id": {"type": "string", "format": "uuid"},
                    "body": {"type": "string"},
                    "metadata": {"type": "object"}
                },
                "required": ["dm_conversation_id", "author_id", "body"]
            }
        }),
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
            "name": "edit_message",
            "description": "Edit a message body (author needs message:post; others need workspace:write).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message_id": {"type": "string", "format": "uuid"},
                    "editor_id": {"type": "string", "format": "uuid"},
                    "body": {"type": "string"},
                    "metadata": {"type": "object"}
                },
                "required": ["message_id", "editor_id", "body"]
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
            "name": "add_reaction",
            "description": "Add an emoji reaction to a message.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid"},
                    "emoji": {"type": "string"}
                },
                "required": ["message_id", "member_id", "emoji"]
            }
        }),
        json!({
            "name": "remove_reaction",
            "description": "Remove an emoji reaction from a message.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid"},
                    "emoji": {"type": "string"}
                },
                "required": ["message_id", "member_id", "emoji"]
            }
        }),
        json!({
            "name": "list_reactions",
            "description": "List emoji reactions on a message.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message_id": {"type": "string", "format": "uuid"}
                },
                "required": ["message_id"]
            }
        }),
        json!({
            "name": "pin_message",
            "description": "Pin a message to a thread.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "message_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["thread_id", "message_id", "member_id"]
            }
        }),
        json!({
            "name": "unpin_message",
            "description": "Unpin a message from a thread.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "message_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["thread_id", "message_id", "member_id"]
            }
        }),
        json!({
            "name": "list_pins",
            "description": "List pinned messages in a thread.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"}
                },
                "required": ["thread_id"]
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
            "name": "begin_artifact_multipart",
            "description": "Start an S3 multipart upload for a large artifact (requires S3 backend).",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "upload_artifact_multipart_part",
            "description": "Upload one part of an in-progress multipart artifact.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "upload_id": {"type": "string"},
                    "object_key": {"type": "string"},
                    "part_number": {"type": "integer", "minimum": 1},
                    "content_base64": {"type": "string"}
                },
                "required": ["upload_id", "object_key", "part_number", "content_base64"]
            }
        }),
        json!({
            "name": "complete_artifact_multipart",
            "description": "Finish multipart upload, content-address bytes, and register artifact metadata.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "upload_id": {"type": "string"},
                    "object_key": {"type": "string"},
                    "parts": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "part_number": {"type": "integer"},
                                "etag": {"type": "string"}
                            },
                            "required": ["part_number", "etag"]
                        }
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["screenshot", "recording", "transcript", "code_dump", "attachment"]
                    },
                    "mime_type": {"type": "string"},
                    "uploaded_by": {"type": "string", "format": "uuid"}
                },
                "required": ["upload_id", "object_key", "parts", "kind"]
            }
        }),
        json!({
            "name": "abort_artifact_multipart",
            "description": "Abort a failed multipart upload.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "upload_id": {"type": "string"},
                    "object_key": {"type": "string"}
                },
                "required": ["upload_id", "object_key"]
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
                    "mode": {
                        "type": "string",
                        "enum": ["lexical", "semantic"],
                        "default": "lexical"
                    },
                    "limit": {"type": "integer", "default": 25},
                    "author_id": {"type": "string", "format": "uuid"},
                    "channel_id": {"type": "string", "format": "uuid"},
                    "kind": {"type": "string", "enum": ["human", "agent"]}
                },
                "required": ["workspace_id", "query"]
            }
        }),
        json!({
            "name": "register_slash_command",
            "description": "Register a workspace slash command handler (http URL or MCP tool name).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "name": {"type": "string"},
                    "description": {"type": "string"},
                    "handler_kind": {"type": "string", "enum": ["http", "mcp_tool"]},
                    "handler_target": {"type": "string"}
                },
                "required": ["workspace_id", "name", "handler_kind", "handler_target"]
            }
        }),
        json!({
            "name": "list_slash_commands",
            "description": "List registered slash commands in a workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"}
                },
                "required": ["workspace_id"]
            }
        }),
    ]
}

pub async fn dispatch(
    store: &Arc<dyn Store>,
    artifacts: &Arc<dyn ArtifactStore>,
    search: &Arc<dyn maidan_search::Search>,
    embedding_provider: &Arc<dyn maidan_search::EmbeddingProvider>,
    auth: &AuthContext,
    name: &str,
    args: &Value,
) -> Result<Value, McpError> {
    match name {
        "list_channels" => list_channels(store, args).await,
        "open_dm_conversation" => open_dm_conversation(store, args).await,
        "list_dm_conversations" => list_dm_conversations(store, args).await,
        "post_dm_message" => post_dm_message(store, args).await,
        "list_threads" => list_threads(store, args).await,
        "list_messages" => list_messages(store, args).await,
        "post_message" => post_message(store, args).await,
        "edit_message" => edit_message(store, auth, args).await,
        "record_mention" => record_mention(store, args).await,
        "cast_vote" => cast_vote(store, args).await,
        "add_reaction" => add_reaction(store, args).await,
        "remove_reaction" => remove_reaction(store, args).await,
        "list_reactions" => list_reactions(store, args).await,
        "pin_message" => pin_message(store, args).await,
        "unpin_message" => unpin_message(store, args).await,
        "list_pins" => list_pins(store, args).await,
        "add_reference" => add_reference(store, args).await,
        "upload_artifact" => upload_artifact(store, artifacts, args).await,
        "begin_artifact_multipart" => begin_artifact_multipart(artifacts).await,
        "upload_artifact_multipart_part" => upload_artifact_multipart_part(artifacts, args).await,
        "complete_artifact_multipart" => complete_artifact_multipart(store, artifacts, args).await,
        "abort_artifact_multipart" => abort_artifact_multipart(artifacts, args).await,
        "get_artifact_metadata" => get_artifact_metadata(store, args).await,
        "search_messages" => search_messages(search, embedding_provider, args).await,
        "register_slash_command" => register_slash_command(store, auth, args).await,
        "list_slash_commands" => list_slash_commands(store, auth, args).await,
        other => Err(McpError::MethodNotFound(format!("tools/{other}"))),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SearchMessagesMode {
    #[default]
    Lexical,
    Semantic,
}

#[derive(Deserialize)]
struct SearchMessagesArgs {
    workspace_id: uuid::Uuid,
    query: String,
    #[serde(default)]
    mode: SearchMessagesMode,
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

fn s3_artifacts(artifacts: &Arc<dyn ArtifactStore>) -> Result<&S3Store, McpError> {
    artifacts
        .as_ref()
        .as_any()
        .downcast_ref::<S3Store>()
        .ok_or_else(|| {
            McpError::InvalidParams(
                "multipart uploads require S3 artifact backend (ARTIFACT_BACKEND=s3)".into(),
            )
        })
}

fn multipart_upload(upload_id: &str, object_key: &str) -> MultipartUpload {
    MultipartUpload {
        upload_id: upload_id.to_string(),
        object_key: object_key.to_string(),
    }
}

async fn begin_artifact_multipart(artifacts: &Arc<dyn ArtifactStore>) -> Result<Value, McpError> {
    let upload = s3_artifacts(artifacts)?
        .begin_multipart_upload()
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    Ok(json!({
        "upload_id": upload.upload_id,
        "object_key": upload.object_key,
    }))
}

#[derive(Deserialize)]
struct UploadMultipartPartArgs {
    upload_id: String,
    object_key: String,
    part_number: i32,
    content_base64: String,
}

async fn upload_artifact_multipart_part(
    artifacts: &Arc<dyn ArtifactStore>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: UploadMultipartPartArgs = serde_json::from_value(args.clone())?;
    let raw = STANDARD
        .decode(&a.content_base64)
        .map_err(|e| McpError::InvalidParams(format!("invalid base64: {e}")))?;
    let upload = multipart_upload(&a.upload_id, &a.object_key);
    let etag = s3_artifacts(artifacts)?
        .upload_part(&upload, a.part_number, Bytes::from(raw))
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    Ok(json!({
        "part_number": a.part_number,
        "etag": etag,
    }))
}

#[derive(Deserialize)]
struct MultipartPartArg {
    part_number: i32,
    etag: String,
}

#[derive(Deserialize)]
struct CompleteMultipartArgs {
    upload_id: String,
    object_key: String,
    parts: Vec<MultipartPartArg>,
    kind: ArtifactKind,
    mime_type: Option<String>,
    uploaded_by: Option<uuid::Uuid>,
}

async fn complete_artifact_multipart(
    store: &Arc<dyn Store>,
    artifacts: &Arc<dyn ArtifactStore>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CompleteMultipartArgs = serde_json::from_value(args.clone())?;
    let upload = multipart_upload(&a.upload_id, &a.object_key);
    let parts: Vec<CompletedPart> = a
        .parts
        .into_iter()
        .map(|p| CompletedPart {
            part_number: p.part_number,
            etag: p.etag,
        })
        .collect();
    let sha = s3_artifacts(artifacts)?
        .complete_multipart_upload(&upload, &parts)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    let bytes = artifacts
        .get(&sha)
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
struct AbortMultipartArgs {
    upload_id: String,
    object_key: String,
}

async fn abort_artifact_multipart(
    artifacts: &Arc<dyn ArtifactStore>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AbortMultipartArgs = serde_json::from_value(args.clone())?;
    let upload = multipart_upload(&a.upload_id, &a.object_key);
    s3_artifacts(artifacts)?
        .abort_multipart_upload(&upload)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;
    Ok(json!({ "aborted": true }))
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
    embedding_provider: &Arc<dyn maidan_search::EmbeddingProvider>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SearchMessagesArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    let filters = maidan_search::SearchFilters {
        author_id: a.author_id.map(maidan_types::MemberId),
        channel_id: a.channel_id.map(maidan_types::ChannelId),
        author_kind: a.kind,
    };
    let hits = match a.mode {
        SearchMessagesMode::Lexical => {
            search
                .search_messages(workspace_id, &a.query, a.limit, &filters)
                .await?
        }
        SearchMessagesMode::Semantic => {
            let embedding = embedding_provider
                .embed(&a.query)
                .map_err(|e| McpError::Internal(format!("embedding generation failed: {e}")))?;
            search
                .semantic_search(
                    workspace_id,
                    &embedding,
                    a.limit,
                    &filters,
                    embedding_provider.model_name(),
                )
                .await?
        }
    };
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
struct OpenDmArgs {
    workspace_id: uuid::Uuid,
    member_id: uuid::Uuid,
    other_member_id: uuid::Uuid,
}

async fn open_dm_conversation(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: OpenDmArgs = serde_json::from_value(args.clone())?;
    let dm = store
        .open_dm_conversation(
            WorkspaceId(a.workspace_id),
            MemberId(a.member_id),
            MemberId(a.other_member_id),
        )
        .await?;
    Ok(content_json(&dm))
}

#[derive(Deserialize)]
struct ListDmArgs {
    workspace_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

async fn list_dm_conversations(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListDmArgs = serde_json::from_value(args.clone())?;
    let list = store
        .list_dm_conversations_for_member(WorkspaceId(a.workspace_id), MemberId(a.member_id))
        .await?;
    Ok(content_json(&list))
}

#[derive(Deserialize)]
struct PostDmMessageArgs {
    dm_conversation_id: uuid::Uuid,
    author_id: uuid::Uuid,
    body: String,
    #[serde(default)]
    metadata: Value,
}

async fn post_dm_message(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: PostDmMessageArgs = serde_json::from_value(args.clone())?;
    let dm = store
        .get_dm_conversation(DmConversationId(a.dm_conversation_id))
        .await?;
    if dm.member_low_id != MemberId(a.author_id) && dm.member_high_id != MemberId(a.author_id) {
        return Err(McpError::InvalidParams(
            "author_id must be a DM participant".into(),
        ));
    }
    let body = a.body.clone();
    let msg = store
        .post_message(NewMessage {
            thread_id: dm.thread_id,
            author_id: MemberId(a.author_id),
            body,
            metadata: if a.metadata.is_null() {
                json!({})
            } else {
                a.metadata
            },
        })
        .await?;
    let _ = route_mentions_for_message(store.as_ref(), msg.id, msg.author_id, &msg.body).await;
    Ok(content_json(&msg))
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
    let body = a.body.clone();
    let msg = store
        .post_message(NewMessage {
            thread_id: ThreadId(a.thread_id),
            author_id: MemberId(a.author_id),
            body,
            metadata: if a.metadata.is_null() {
                json!({})
            } else {
                a.metadata
            },
        })
        .await?;
    let _ = route_mentions_for_message(store.as_ref(), msg.id, msg.author_id, &msg.body).await;
    Ok(content_json(&msg))
}

#[derive(Deserialize)]
struct EditMessageArgs {
    message_id: uuid::Uuid,
    editor_id: uuid::Uuid,
    body: String,
    #[serde(default)]
    metadata: Option<Value>,
}

async fn edit_message(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: EditMessageArgs = serde_json::from_value(args.clone())?;
    let message_id = MessageId(a.message_id);
    let existing = store.get_message(message_id).await?;
    if existing.tombstoned_at.is_some() {
        return Err(McpError::InvalidParams("message is tombstoned".into()));
    }
    let editor_id = MemberId(a.editor_id);
    if !auth.bypass {
        if editor_id == existing.author_id {
            auth.require_capability(MESSAGE_POST)
                .map_err(McpError::from)?;
        } else {
            auth.require_capability(WORKSPACE_WRITE)
                .map_err(McpError::from)?;
        }
    }
    let metadata = match a.metadata {
        Some(v) if !v.is_null() => v,
        _ => existing.metadata,
    };
    let msg = store
        .edit_message(
            message_id,
            editor_id,
            EditMessage {
                body: a.body,
                metadata,
            },
        )
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
struct ReactionArgs {
    message_id: uuid::Uuid,
    member_id: uuid::Uuid,
    emoji: String,
}

async fn add_reaction(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ReactionArgs = serde_json::from_value(args.clone())?;
    store
        .add_reaction(NewReaction {
            message_id: MessageId(a.message_id),
            member_id: MemberId(a.member_id),
            emoji: a.emoji,
        })
        .await?;
    Ok(content_json(&json!({"ok": true})))
}

async fn remove_reaction(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ReactionArgs = serde_json::from_value(args.clone())?;
    let removed = store
        .remove_reaction(MessageId(a.message_id), MemberId(a.member_id), &a.emoji)
        .await?;
    Ok(content_json(&json!({"removed": removed})))
}

#[derive(Deserialize)]
struct ListReactionsArgs {
    message_id: uuid::Uuid,
}

async fn list_reactions(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListReactionsArgs = serde_json::from_value(args.clone())?;
    let list = store
        .list_reactions_for_message(MessageId(a.message_id))
        .await?;
    Ok(content_json(&list))
}

#[derive(Deserialize)]
struct PinArgs {
    thread_id: uuid::Uuid,
    message_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

async fn pin_message(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: PinArgs = serde_json::from_value(args.clone())?;
    store
        .pin_message(NewPin {
            thread_id: ThreadId(a.thread_id),
            message_id: MessageId(a.message_id),
            member_id: MemberId(a.member_id),
        })
        .await?;
    Ok(content_json(&json!({"ok": true})))
}

async fn unpin_message(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: PinArgs = serde_json::from_value(args.clone())?;
    let removed = store
        .unpin_message(ThreadId(a.thread_id), MessageId(a.message_id))
        .await?;
    Ok(content_json(&json!({"removed": removed})))
}

#[derive(Deserialize)]
struct ListPinsArgs {
    thread_id: uuid::Uuid,
}

async fn list_pins(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListPinsArgs = serde_json::from_value(args.clone())?;
    let list = store.list_pins_for_thread(ThreadId(a.thread_id)).await?;
    Ok(content_json(&list))
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

#[derive(Deserialize)]
struct RegisterSlashCommandArgs {
    workspace_id: uuid::Uuid,
    name: String,
    description: Option<String>,
    handler_kind: String,
    handler_target: String,
}

async fn register_slash_command(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    if !auth.bypass {
        auth.require_capability(WORKSPACE_WRITE)
            .map_err(McpError::from)?;
    }
    let a: RegisterSlashCommandArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)
        .map_err(McpError::from)?;
    let name = normalize_slash_name(&a.name)?;
    let handler_kind = SlashHandlerKind::parse(&a.handler_kind)
        .ok_or_else(|| McpError::InvalidParams("handler_kind must be http or mcp_tool".into()))?;
    match handler_kind {
        SlashHandlerKind::Http => validate_http_target(&a.handler_target)?,
        SlashHandlerKind::McpTool => {
            required_capability(&a.handler_target)?;
        }
    }
    let secret_ciphertext = if handler_kind == SlashHandlerKind::Http {
        let key = maidan_auth::encryption_key_from_env().map_err(|_| {
            McpError::InvalidParams(
                "FEDERATION_ENCRYPTION_KEY must be set for http slash handlers".into(),
            )
        })?;
        let secret = TokenSecret::generate();
        let ciphertext = encrypt_peer_secret(secret.as_str(), &key)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        let command = store
            .create_slash_command(NewSlashCommand {
                workspace_id,
                name,
                description: a.description,
                handler_kind,
                handler_target: a.handler_target.trim().to_string(),
                secret_ciphertext: ciphertext,
            })
            .await
            .map_err(McpError::from)?;
        return Ok(content_json(&json!({
            "command": command,
            "secret": secret.as_str()
        })));
    } else {
        String::new()
    };
    let command = store
        .create_slash_command(NewSlashCommand {
            workspace_id,
            name,
            description: a.description,
            handler_kind,
            handler_target: a.handler_target.trim().to_string(),
            secret_ciphertext,
        })
        .await
        .map_err(McpError::from)?;
    Ok(content_json(&command))
}

#[derive(Deserialize)]
struct ListSlashCommandsArgs {
    workspace_id: uuid::Uuid,
}

async fn list_slash_commands(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    if !auth.bypass {
        auth.require_capability(WORKSPACE_READ)
            .map_err(McpError::from)?;
    }
    let a: ListSlashCommandsArgs = serde_json::from_value(args.clone())?;
    let workspace_id = WorkspaceId(a.workspace_id);
    auth.ensure_workspace(workspace_id)
        .map_err(McpError::from)?;
    let commands = store
        .list_slash_commands(workspace_id)
        .await
        .map_err(McpError::from)?;
    Ok(content_json(&commands))
}

fn normalize_slash_name(name: &str) -> Result<String, McpError> {
    let normalized = name.trim().trim_start_matches('/').to_ascii_lowercase();
    if normalized.is_empty() || normalized.len() > 32 {
        return Err(McpError::InvalidParams(
            "slash command name must be 1-32 characters".into(),
        ));
    }
    if !normalized
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(McpError::InvalidParams(
            "slash command name may only contain a-z, 0-9, _, -".into(),
        ));
    }
    Ok(normalized)
}

fn validate_http_target(url: &str) -> Result<(), McpError> {
    let trimmed = url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(McpError::InvalidParams(
            "handler_target url must use http or https".into(),
        ));
    }
    if trimmed.len() > 2048 || trimmed.as_bytes().contains(&b' ') {
        return Err(McpError::InvalidParams("invalid handler url".into()));
    }
    Ok(())
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
