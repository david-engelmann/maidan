//! Catalog of every tool the MCP server exposes. The JSON-RPC client
//! receives this verbatim in the `tools/list` response.

use serde_json::{json, Value};

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
                    "body": {"type": "string", "description": "plain text; omit when sending typed content (body is derived from it)"},
                    "metadata": {"type": "object"},
                    "content": {"type": "array", "items": {"type": "object"}, "description": "typed content blocks: {type: text|code|tool_use|tool_result|resource_link, ...}"}
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
            "name": "add_channel_member",
            "description": "Add (or update the role of) a member of a channel. Requires channel:admin. Private channels are gated to their members.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid"},
                    "role": {"type": "string", "enum": ["member", "admin"], "default": "member"}
                },
                "required": ["channel_id", "member_id"]
            }
        }),
        json!({
            "name": "list_channel_members",
            "description": "List the members of a channel. Requires channel:admin.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel_id": {"type": "string", "format": "uuid"}
                },
                "required": ["channel_id"]
            }
        }),
        json!({
            "name": "remove_channel_member",
            "description": "Remove a member from a channel. Requires channel:admin.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["channel_id", "member_id"]
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
            "name": "assign_thread",
            "description": "Assign or hand off a thread/task to a member.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "actor_id": {"type": "string", "format": "uuid", "description": "member performing the assignment"},
                    "assignee_id": {"type": "string", "format": "uuid", "description": "member to assign the thread to"}
                },
                "required": ["thread_id", "actor_id", "assignee_id"]
            }
        }),
        json!({
            "name": "claim_thread",
            "description": "Atomically claim an unassigned thread for a member. Returns {thread, claimed}; claimed=false if it was already assigned.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid", "description": "member claiming the thread"}
                },
                "required": ["thread_id", "member_id"]
            }
        }),
        json!({
            "name": "unassign_thread",
            "description": "Clear a thread's assignee.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "actor_id": {"type": "string", "format": "uuid", "description": "member performing the unassignment"}
                },
                "required": ["thread_id", "actor_id"]
            }
        }),
        json!({
            "name": "list_assigned_threads",
            "description": "List the threads currently assigned to a member (their work queue), oldest first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "claim_next_thread",
            "description": "Atomically claim the oldest claimable thread in a channel for a member (claimable = unassigned or its lease expired). Returns the claimed thread, or null when there is no claimable work.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid", "description": "member to claim the thread for"},
                    "lease_secs": {"type": "integer", "description": "optional lease deadline in seconds; the claim is reclaimable after it lapses (omit for a durable claim)"}
                },
                "required": ["channel_id", "member_id"]
            }
        }),
        json!({
            "name": "renew_claim",
            "description": "Extend a claimed thread's lease (heartbeat). Only the current assignee may renew.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid", "description": "the current assignee"},
                    "lease_secs": {"type": "integer", "description": "new lease deadline in seconds from now"}
                },
                "required": ["thread_id", "member_id", "lease_secs"]
            }
        }),
        json!({
            "name": "list_mentions",
            "description": "List recent @mentions of a member (most recent first).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "limit": {"type": "integer", "description": "max results (default 50, max 500)"}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "get_inbox",
            "description": "A member's mention inbox: recent mentions plus the read-cursor, so an agent can find what it hasn't seen.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "limit": {"type": "integer", "description": "max mentions (default 50, max 500)"}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "mark_inbox_read",
            "description": "Advance a member's inbox read-cursor through an instant (RFC 3339); returns the updated inbox.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "read_through": {"type": "string", "format": "date-time"}
                },
                "required": ["member_id", "read_through"]
            }
        }),
        json!({
            "name": "list_messages",
            "description": "List messages in a thread.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "limit": {"type": "integer", "default": 100, "minimum": 1, "maximum": 500}
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
                    "body": {"type": "string", "description": "plain text; omit when sending typed content (body is derived from it)"},
                    "metadata": {"type": "object"},
                    "content": {"type": "array", "items": {"type": "object"}, "description": "typed content blocks: {type: text|code|tool_use|tool_result|resource_link, ...}"}
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
                    "body": {"type": "string", "description": "plain text; omit when sending typed content (body is derived from it)"},
                    "metadata": {"type": "object"},
                    "content": {"type": "array", "items": {"type": "object"}, "description": "typed content blocks: {type: text|code|tool_use|tool_result|resource_link, ...}"}
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
            "description": "Full-text, semantic, or hybrid search over a workspace's messages. Returns ranked hits with highlighted snippets.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "query": {"type": "string", "minLength": 1},
                    "mode": {
                        "type": "string",
                        "enum": ["lexical", "semantic", "hybrid"],
                        "default": "lexical"
                    },
                    "limit": {"type": "integer", "default": 25},
                    "snippet_only": {"type": "boolean", "default": false, "description": "Drop full message body from each hit (keep only the snippet) to save tokens."},
                    "author_id": {"type": "string", "format": "uuid"},
                    "channel_id": {"type": "string", "format": "uuid"},
                    "kind": {"type": "string", "enum": ["human", "agent"]},
                    "embedding_model": {
                        "type": "string",
                        "description": "Semantic/hybrid only: registered model name (default: active provider)."
                    },
                    "hybrid_weight": {
                        "type": "number",
                        "description": "Hybrid only: semantic weight in [0,1] (default 0.5). combined = w*semantic + (1-w)*lexical over normalized scores."
                    }
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
        json!({
            "name": "register_fsm_hook",
            "description": "Register an FSM hook invoked on matching thread state transitions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "label": {"type": "string"},
                    "from_state": {"type": "string", "enum": ["open", "in_review", "closed", "archived"]},
                    "to_state": {"type": "string", "enum": ["open", "in_review", "closed", "archived"]},
                    "handler_kind": {"type": "string", "enum": ["http", "mcp_tool"]},
                    "handler_target": {"type": "string"}
                },
                "required": ["workspace_id", "handler_kind", "handler_target"]
            }
        }),
        json!({
            "name": "list_fsm_hooks",
            "description": "List registered FSM automation hooks in a workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"}
                },
                "required": ["workspace_id"]
            }
        }),
        json!({
            "name": "get_thread_context",
            "description": "Pack thread messages, edits, references, and FSM history for agent prompts. Edits are lean by default (id/editor/timestamp only); pass include_edits=true for full before/after bodies.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "message_limit": {"type": "integer", "minimum": 1, "maximum": 500},
                    "transition_limit": {"type": "integer", "minimum": 1, "maximum": 200},
                    "include_edits": {"type": "boolean", "default": false, "description": "Include full body_before/body_after on each edit (heavy); default returns edit metadata only."}
                },
                "required": ["thread_id"]
            }
        }),
        json!({
            "name": "get_workspace_context",
            "description": "Pack workspace channels and thread contexts (bounded by thread_limit).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "thread_limit": {"type": "integer", "minimum": 1, "maximum": 50},
                    "message_limit": {"type": "integer", "minimum": 1, "maximum": 500},
                    "transition_limit": {"type": "integer", "minimum": 1, "maximum": 200}
                },
                "required": ["workspace_id"]
            }
        }),
        json!({
            "name": "summarize_thread",
            "description": "Summarize a thread by asking the connected MCP client to sample an LLM (server→client sampling/createMessage over the GET /mcp/streamable stream). Requires a streamable session whose client declared the sampling capability.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 500, "default": 50},
                    "instructions": {"type": "string", "description": "Optional steer for the summary."}
                },
                "required": ["thread_id"]
            }
        }),
        json!({
            "name": "request_approval",
            "description": "Human-in-the-loop gate: ask the human on the connected MCP client to approve or reject an action (server→client elicitation/create over the GET /mcp/streamable stream). Requires a streamable session whose client declared the elicitation capability. Returns {approved, action, content}.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "description": "what the human is being asked to approve"},
                    "schema": {"type": "object", "description": "optional JSON Schema for structured detail the human may supply (MCP requestedSchema)"}
                },
                "required": ["prompt"]
            }
        }),
        json!({
            "name": "list_roots",
            "description": "List the roots (filesystem/workspace boundaries) the connected MCP client exposes, via the server→client roots/list request over the GET /mcp/streamable stream. Requires a streamable session whose client declared the roots capability. Returns the client's roots array.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        }),
    ]
}
