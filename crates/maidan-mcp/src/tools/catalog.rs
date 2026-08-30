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
            "name": "get_tool_transcript",
            "description": "A thread's tool-call transcript: every ToolUse block correlated with its ToolResult by id. A token-lean projection that drops text/code blocks and bodies.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "limit": {"type": "integer", "default": 200, "minimum": 1, "maximum": 500, "description": "max messages to scan"}
                },
                "required": ["thread_id"]
            }
        }),
        json!({
            "name": "assign_thread",
            "description": "Assign or hand off a thread/task to a member, optionally with a handoff note delivered to subscribers on the assignment event.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "actor_id": {"type": "string", "format": "uuid", "description": "member performing the assignment"},
                    "assignee_id": {"type": "string", "format": "uuid", "description": "member to assign the thread to"},
                    "note": {"type": "string", "description": "optional handoff note for the assignee"}
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
            "name": "add_thread_dependency",
            "description": "Add a task-dependency edge: the thread depends on depends_on_thread_id and stays blocked (won't be handed out by claim_next) until that dependency reaches a terminal state. Both threads must be in the same workspace.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid", "description": "the dependent task"},
                    "depends_on_thread_id": {"type": "string", "format": "uuid", "description": "the task it depends on"}
                },
                "required": ["thread_id", "depends_on_thread_id"]
            }
        }),
        json!({
            "name": "list_thread_dependencies",
            "description": "List a task's dependencies plus whether it is ready to run (true when every dependency is terminal).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"}
                },
                "required": ["thread_id"]
            }
        }),
        json!({
            "name": "get_queue_depth",
            "description": "A channel's task-queue depth: counts of its open task threads as {open, ready, assigned, blocked}, for deciding whether to scale workers. ready is what claim_next_thread could take now.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel_id": {"type": "string", "format": "uuid"}
                },
                "required": ["channel_id"]
            }
        }),
        json!({
            "name": "set_thread_result",
            "description": "Attach a task's structured result (arbitrary JSON). Upserts one result per thread and notifies waiters via a thread_result_set event. Use when finishing a task so a requester or parent can read the output.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "result": {"type": "object", "description": "structured JSON result payload (an object)"}
                },
                "required": ["thread_id", "result"]
            }
        }),
        json!({
            "name": "get_thread_result",
            "description": "Read a task's structured result, or null if none has been produced yet.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"}
                },
                "required": ["thread_id"]
            }
        }),
        json!({
            "name": "wait_for_result",
            "description": "Block until a task's result is produced (a thread_result_set event for thread_id), returning the result payload, or null on timeout. The coordination wait for spawn/wait/aggregate. Live-only: read get_thread_result first for an already-produced result.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "timeout_ms": {"type": "integer", "description": "wait window ms (default 30000, clamped 1000-300000)"}
                },
                "required": ["thread_id"]
            }
        }),
        json!({
            "name": "get_dependency_results",
            "description": "Gather the structured results of a parent task's dependencies as a list of {thread_id, result} objects (result null if not produced yet), skipping dependencies you can't access. The spawn/wait/aggregate read for a parent task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid", "description": "the parent task"}
                },
                "required": ["thread_id"]
            }
        }),
        json!({
            "name": "add_member_skill",
            "description": "Declare a skill (free-form tag) for a member. Skill routing gates claim_next: a task is claimable by a member only if it holds all the task's required skills.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "skill": {"type": "string"}
                },
                "required": ["member_id", "skill"]
            }
        }),
        json!({
            "name": "list_member_skills",
            "description": "List a member's declared skills.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "add_thread_required_skill",
            "description": "Add a required skill to a task. Only a member holding every required skill can claim the task via claim_next_thread.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "skill": {"type": "string"}
                },
                "required": ["thread_id", "skill"]
            }
        }),
        json!({
            "name": "list_thread_required_skills",
            "description": "List a task's required skills.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"}
                },
                "required": ["thread_id"]
            }
        }),
        json!({
            "name": "create_task_schedule",
            "description": "Create a task schedule: when due, the sweeper creates a thread titled `title` in `channel_id`. interval_secs omitted = one-shot; a positive value = recurring. first_run_at omitted = fire on the next tick.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel_id": {"type": "string", "format": "uuid"},
                    "title": {"type": "string"},
                    "interval_secs": {"type": "integer", "description": "recurrence period in seconds; omit for a one-shot"},
                    "first_run_at": {"type": "string", "format": "date-time", "description": "when to first fire (default: now)"}
                },
                "required": ["channel_id", "title"]
            }
        }),
        json!({
            "name": "list_task_schedules",
            "description": "List the caller's workspace task schedules (filtered to channels the caller can access).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "set_glossary_term",
            "description": "Define (or redefine) a term in the workspace's shared glossary — the canonical term -> definition so agents use words the same way (the anti-drift pin; the target of a `defines` reference). Upserts on the term.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "term": {"type": "string"},
                    "definition": {"type": "string"},
                    "aliases": {"type": "array", "items": {"type": "string"}, "description": "alternate labels for the same term"}
                },
                "required": ["term", "definition"]
            }
        }),
        json!({
            "name": "get_glossary_term",
            "description": "Look up one term's canonical definition in the workspace glossary. Returns null when the term is undefined.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "term": {"type": "string"}
                },
                "required": ["term"]
            }
        }),
        json!({
            "name": "list_glossary_terms",
            "description": "List all defined terms in the workspace's shared glossary, ordered by term.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "wait_for_ready",
            "description": "Block until a task becomes ready (its last blocking dependency reaches a terminal state, emitting thread_ready), or the timeout lapses. Returns the ThreadReady event, or null on timeout. Scoped to channel_id when given, else any accessible thread in the workspace. Live-only: it sees readiness signalled after the call subscribes, so pick up already-ready work with claim_next_thread first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel_id": {"type": "string", "format": "uuid", "description": "optional: scope to one channel's tasks"},
                    "timeout_ms": {"type": "integer", "default": 30000, "minimum": 1, "maximum": 300000, "description": "long-poll window in milliseconds"}
                }
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
            "name": "wait_for_mention",
            "description": "Block until the member is next @mentioned, or the timeout lapses. Returns the mention event, or null on timeout. Live-only: it sees mentions recorded after the call subscribes, so drain existing ones with get_inbox first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "timeout_ms": {"type": "integer", "default": 30000, "minimum": 1, "maximum": 300000, "description": "long-poll window in milliseconds"}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "list_notifications",
            "description": "List a member's per-recipient notifications, newest first. Set unread_only to see just the unread ones. The durable inbox the notification router fills; drain it here, then wait_for_notification for new ones.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "unread_only": {"type": "boolean", "default": false},
                    "limit": {"type": "integer", "default": 50, "minimum": 1, "maximum": 500}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "get_unread_count",
            "description": "A member's unread-notification badge count.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "mark_notification_read",
            "description": "Mark one of a member's notifications read (recipient-scoped; marked=false if the id isn't this member's).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "notification_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id", "notification_id"]
            }
        }),
        json!({
            "name": "wait_for_notification",
            "description": "Block until the member gets a new notification-worthy event (today: mentions), or the timeout lapses. The general form of wait_for_mention. Returns the triggering event, or null on timeout. Live-only: drain existing notifications with list_notifications first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "timeout_ms": {"type": "integer", "default": 30000, "minimum": 1, "maximum": 300000, "description": "long-poll window in milliseconds"}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "set_notification_pref",
            "description": "Set a member's mute preference for an event kind (kind is snake_case, e.g. mention_recorded). When muted, the router stops writing notifications of that kind for this member.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "kind": {"type": "string", "description": "event kind, snake_case"},
                    "muted": {"type": "boolean"}
                },
                "required": ["member_id", "kind", "muted"]
            }
        }),
        json!({
            "name": "list_notification_prefs",
            "description": "List a member's notification preferences (per-kind mute flags).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "set_delivery_mode",
            "description": "Set a member's email delivery mode: immediate (a per-notification email) or digest (a periodic rollup instead). The two are mutually exclusive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "mode": {"type": "string", "enum": ["immediate", "digest"]}
                },
                "required": ["member_id", "mode"]
            }
        }),
        json!({
            "name": "get_delivery_mode",
            "description": "Get a member's email delivery mode (immediate when never set).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "set_member_email",
            "description": "Set a member's delivery email address (where their email notifications go). A light @ check; full validation happens at send.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "email": {"type": "string"}
                },
                "required": ["member_id", "email"]
            }
        }),
        json!({
            "name": "get_member_email",
            "description": "Get a member's delivery email address (null when unset).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "delete_member_email",
            "description": "Clear a member's delivery email address (opt out of email). Returns {deleted}.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "follow_channel",
            "description": "Follow a channel so the member is notified of new messages there even without a mention (honors mutes). Requires access to the channel.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "channel_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id", "channel_id"]
            }
        }),
        json!({
            "name": "unfollow_channel",
            "description": "Stop following a channel (removed=false if not following).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "channel_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id", "channel_id"]
            }
        }),
        json!({
            "name": "list_channel_follows",
            "description": "List the channels a member follows.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id"]
            }
        }),
        json!({
            "name": "follow_thread",
            "description": "Follow a thread so the member is notified of new messages in it even without a mention (honors mutes). Requires access to the thread.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "thread_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id", "thread_id"]
            }
        }),
        json!({
            "name": "unfollow_thread",
            "description": "Stop following a thread (removed=false if not following).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"},
                    "thread_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id", "thread_id"]
            }
        }),
        json!({
            "name": "list_thread_follows",
            "description": "List the threads a member follows.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_id": {"type": "string", "format": "uuid"}
                },
                "required": ["member_id"]
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
            "name": "seed_from_message",
            "description": "Seed a new titled work thread from a source message (the write side of 're-ask'), linked by a seeded_from reference edge. inclusion: 'pointer' (default, edge only) or 'quote' (a first message quoting the source). The source is untouched; N seeds per source. Lineage is queryable via list_references (dst=the source, relation=seeded_from).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message_id": {"type": "string", "format": "uuid", "description": "the source message"},
                    "title": {"type": "string"},
                    "inclusion": {"type": "string", "enum": ["pointer", "quote"], "default": "pointer"},
                    "channel_id": {"type": "string", "format": "uuid", "description": "target channel (default: the source's channel)"}
                },
                "required": ["message_id", "title"]
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
            "description": "Cast a vote on a message (e.g. approve, request-changes, emoji). Optional confidence (0..1) for weighted consensus; re-casting the same kind updates your confidence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "message_id": {"type": "string", "format": "uuid"},
                    "member_id": {"type": "string", "format": "uuid"},
                    "kind": {"type": "string"},
                    "confidence": {"type": "number", "minimum": 0, "maximum": 1, "description": "optional confidence weight for weighted consensus"}
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
                    "relation": {"type": "string", "description": "typed relation; controlled set: supports/refutes/defines/depends/duplicates/grounds/supersedes (other values are allowed and round-trip verbatim)"}
                },
                "required": ["src_kind", "src_id", "dst_kind", "dst_id", "relation"]
            }
        }),
        json!({
            "name": "list_references",
            "description": "List references FROM a source (forward) or TO a target (reverse — 'what references this'), optionally filtered by relation. Provide exactly one of the src_kind+src_id or dst_kind+dst_id pair.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "src_kind": {"type": "string", "enum": ["thread", "message"]},
                    "src_id": {"type": "string", "format": "uuid"},
                    "dst_kind": {"type": "string", "enum": ["thread", "message"]},
                    "dst_id": {"type": "string", "format": "uuid"},
                    "relation": {"type": "string", "description": "optional relation filter (controlled set: supports/refutes/defines/depends/duplicates/grounds/supersedes, or any custom value)"}
                }
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
            "description": "Pack thread messages, edits, references, FSM history, and the workspace glossary for agent prompts. Edits are lean by default (id/editor/timestamp only); pass include_edits=true for full before/after bodies. The glossary (canonical term definitions) is included by default when non-empty; pass include_glossary=false to drop it. Pass as_of=<event_id> to replay the thread as it stood at that event-log id (deterministic over the immutable log; audit / re-ask from before a tangent).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "message_limit": {"type": "integer", "minimum": 1, "maximum": 500},
                    "transition_limit": {"type": "integer", "minimum": 1, "maximum": 200},
                    "include_edits": {"type": "boolean", "default": false, "description": "Include full body_before/body_after on each edit (heavy); default returns edit metadata only."},
                    "include_glossary": {"type": "boolean", "default": true, "description": "Include the workspace glossary (grounding); omitted when empty. Set false for a token-tight pack."},
                    "as_of": {"type": "integer", "description": "Event-log id: reconstruct the thread as it stood at that point (as-of replay). Omit for the live pack."}
                },
                "required": ["thread_id"]
            }
        }),
        json!({
            "name": "snapshot_thread_context",
            "description": "Freeze the assembled context pack (live or as_of) into the content-addressed artifact store — a tamper-evident, deduped record of exactly what the agent was handed. Same params as get_thread_context; returns the artifact (kind=context_snapshot). Requires artifact:upload. Fetch the bytes via the artifact sha.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "format": "uuid"},
                    "message_limit": {"type": "integer", "minimum": 1, "maximum": 500},
                    "transition_limit": {"type": "integer", "minimum": 1, "maximum": 200},
                    "include_edits": {"type": "boolean", "default": false},
                    "include_glossary": {"type": "boolean", "default": true},
                    "as_of": {"type": "integer", "description": "Event-log id: freeze the thread as it stood at that point. Omit for the live pack."}
                },
                "required": ["thread_id"]
            }
        }),
        json!({
            "name": "get_workspace_context",
            "description": "Pack workspace channels, thread contexts (bounded by thread_limit), and the workspace glossary (once at the top level).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "thread_limit": {"type": "integer", "minimum": 1, "maximum": 50},
                    "message_limit": {"type": "integer", "minimum": 1, "maximum": 500},
                    "transition_limit": {"type": "integer", "minimum": 1, "maximum": 200},
                    "include_glossary": {"type": "boolean", "default": true, "description": "Include the workspace glossary once at the top level (grounding); omitted when empty. Set false to drop it."}
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
