# MCP reference

Auto-generated from `maidan-mcp` `tools/list`, `resources/templates/list`, and `prompts/list` catalogs. Regenerate with `cargo run -p maidan-mcp --bin gen-mcp-reference`.

## Transport

- **Protocol revisions:** `2026-07-28` (default), `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`. `initialize` echoes the revision you request if it is one of these
- **HTTP:** `POST /mcp` (JSON-RPC 2.0)
- **HTTP notifications:** `GET /mcp/notifications` (SSE JSON-RPC notifications)
- **Streamable HTTP:** `POST /mcp/streamable` — every revision from `2025-03-26` on is stateless: one JSON-RPC response per POST, no `Mcp-Session-Id`, a notification answered `202`; optional SEP-2243 `Mcp-Method`/`Mcp-Name` routing headers. Only a `2024-11-05` client (by `initialize` or `MCP-Protocol-Version`) gets the SSE-session model (the first request opens the SSE + `Mcp-Session-Id`; follow-ups with that id are pushed to the session). Server→client messages ride `GET /mcp/streamable` or `GET /mcp/stream`
- **SSE:** `GET /mcp/stream` for workspace event stream replay/live
- **stdio:** `maidan mcp-stdio` for desktop clients (SQLite or Postgres `DATABASE_URL`; `resources/subscribe` notifications). Set `MAIDAN_MCP_TOKEN`: it scopes every tool the process serves, and without it the command refuses unless `--allow-insecure-no-auth` is passed

Bearer token required unless `AUTH_DISABLED=1`.

## JSON-RPC methods

- `initialize`
- `tools/list`, `tools/call`
- `resources/list`, `resources/templates/list`, `resources/read`, `resources/subscribe`, `resources/unsubscribe`
- `prompts/list`, `prompts/get`

**Notification:** `notifications/resources/updated` with `{ "uri": "maidan://..." }` (stdio after each response; HTTP via `GET /mcp/notifications` or `POST /mcp/streamable`). Mutating tools fan out to related thread/channel/workspace/artifact URIs.

## Tools

### `whoami`

Return the authentication-bound identity: actor_id, member_id, optional delegation_grant_id, workspace_id, capabilities, capability_sets the caller fully holds, and whether the credential is a bearer. Call this first — writes are attributed to member_id.

**Capability:** `workspace:read`

```json
{
  "properties": {},
  "type": "object"
}
```

### `list_capability_sets`

List named capability sets (maidan.agent.worker, maidan.human.admin) and the atomic capabilities each expands to at mint time.

**Capability:** `workspace:read`

```json
{
  "properties": {},
  "type": "object"
}
```

### `parse_maidan_uri`

Parse a hierarchical maidan:// room URI (workspace UUID authority, then channels, threads, messages). The authority must be a workspace UUID, not a handle. Optional sha256 fragment is a content hash. MCP thread resource URIs and event pins are rejected.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "uri": {
      "type": "string"
    }
  },
  "required": [
    "uri"
  ],
  "type": "object"
}
```

### `get_room`

Get the authenticated room card for a workspace: stable UUID URI plus the current handle alias. A handle rename does not change the URI.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id"
  ],
  "type": "object"
}
```

### `set_workspace_handle`

Set or rename a workspace handle alias. Stored ids and maidan:// URIs keep using the workspace UUID. Requires workspace:write.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "handle": {
      "type": "string"
    },
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id",
    "handle"
  ],
  "type": "object"
}
```

### `attenuate_token`

Derive a weaker API token from the caller's grant without token:admin (Levy/Madden attenuation). capabilities must be a non-empty subset of what the caller holds. A derived expires_at cannot outlive the parent bearer. Returns the new token secret once.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "capabilities": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "expires_at": {
      "format": "date-time",
      "type": "string"
    },
    "label": {
      "type": "string"
    }
  },
  "required": [
    "capabilities"
  ],
  "type": "object"
}
```

### `delegate_token`

Exchange a durable delegation grant for a short-lived token acting as its subject. The token defaults to 15 minutes, cannot exceed one hour or its grant/parent bearer, and is limited to the intersection of grant and delegate capabilities.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "capabilities": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "expires_at": {
      "format": "date-time",
      "type": "string"
    },
    "grant_id": {
      "format": "uuid",
      "type": "string"
    },
    "label": {
      "type": "string"
    }
  },
  "required": [
    "grant_id"
  ],
  "type": "object"
}
```

### `create_delegation_grant`

Create an expiring capability-scoped grant authorizing one workspace member to delegate actions for another. Requires token:admin and a non-empty purpose.

**Capability:** `token:admin`

```json
{
  "properties": {
    "capabilities": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "delegate_id": {
      "format": "uuid",
      "type": "string"
    },
    "expires_at": {
      "format": "date-time",
      "type": "string"
    },
    "purpose": {
      "type": "string"
    },
    "subject_id": {
      "format": "uuid",
      "type": "string"
    },
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id",
    "subject_id",
    "delegate_id",
    "capabilities",
    "purpose",
    "expires_at"
  ],
  "type": "object"
}
```

### `list_delegation_grants`

List delegation grants in a workspace, including expiry and revocation state. Requires token:admin.

**Capability:** `token:admin`

```json
{
  "properties": {
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id"
  ],
  "type": "object"
}
```

### `revoke_delegation_grant`

Revoke a delegation grant and every exchanged token and attenuated descendant. Requires token:admin.

**Capability:** `token:admin`

```json
{
  "properties": {
    "grant_id": {
      "format": "uuid",
      "type": "string"
    },
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id",
    "grant_id"
  ],
  "type": "object"
}
```

### `open_dm_conversation`

Open or fetch a 1:1 DM conversation between the authenticated member and another workspace member.

**Capability:** `message:post`

```json
{
  "properties": {
    "other_member_id": {
      "format": "uuid",
      "type": "string"
    },
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id",
    "other_member_id"
  ],
  "type": "object"
}
```

### `list_dm_conversations`

List DM conversations for a member in a workspace.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id",
    "member_id"
  ],
  "type": "object"
}
```

### `post_dm_message`

Post a message in a DM conversation.

**Capability:** `message:post`

```json
{
  "properties": {
    "body": {
      "description": "plain text; omit when sending typed content (body is derived from it)",
      "type": "string"
    },
    "content": {
      "description": "typed content blocks: {type: text|code|tool_use|tool_result|resource_link, ...}",
      "items": {
        "type": "object"
      },
      "type": "array"
    },
    "dm_conversation_id": {
      "format": "uuid",
      "type": "string"
    },
    "metadata": {
      "type": "object"
    }
  },
  "required": [
    "dm_conversation_id",
    "body"
  ],
  "type": "object"
}
```

### `list_channels`

List channels in a workspace.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id"
  ],
  "type": "object"
}
```

### `add_channel_member`

Add (or update the role of) a member of a channel. Requires channel:admin. Private channels are gated to their members.

**Capability:** `channel:admin`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "role": {
      "default": "member",
      "enum": [
        "member",
        "admin"
      ],
      "type": "string"
    }
  },
  "required": [
    "channel_id",
    "member_id"
  ],
  "type": "object"
}
```

### `list_channel_members`

List the members of a channel. Requires channel:admin.

**Capability:** `channel:admin`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `remove_channel_member`

Remove a member from a channel. Requires channel:admin.

**Capability:** `channel:admin`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "channel_id",
    "member_id"
  ],
  "type": "object"
}
```

### `list_threads`

List a channel's live threads, oldest first, keyset-paginated. Default 100 (max 500); pass cursor=<last thread id of the prior page> for the next page.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "cursor": {
      "description": "Exclusive keyset cursor: the prior page's last thread id.",
      "format": "uuid",
      "type": "string"
    },
    "limit": {
      "default": 100,
      "description": "Max threads to return (clamped 1..=500).",
      "type": "integer"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `list_child_threads`

A parent thread's child threads, each collapsed to a summary with a message count — a threaded view of 'N replies' per child without loading each child's messages.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "description": "The parent thread id.",
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `list_recently_active_threads`

A channel's threads ordered by last activity — most-recently-posted first. A post floats its thread to the top; a rename does not.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "limit": {
      "default": 50,
      "description": "Max threads to return (clamped 1..=200).",
      "type": "integer"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `mute_thread`

Mute a thread for yourself — the notification router stops routing this thread's activity to you, without leaving the channel or thread. Idempotent.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `unmute_thread`

Unmute a thread you previously muted. Returns unmuted=false if it was not muted.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `mute_channel`

Mute a whole channel for yourself — the notification router stops routing its firehose (new-message notifications) to you, without leaving the channel. A mention still breaks through. Idempotent.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `unmute_channel`

Unmute a channel you previously muted. Returns unmuted=false if it was not muted.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `set_thread_budget`

REPLACE a thread's whole budget envelope. Every dimension must be stated — max_tokens, max_usd_micros ($1 = 1000000), max_turns, max_wall_secs — and null means no cap on that dimension. Omitting one is an error rather than a silent removal, because a removed cap never binds and the run it should have stopped keeps going. Use update_thread_budget to change some dimensions and leave the rest alone. An unrecognized key is rejected rather than ignored. Accumulated usage is preserved. When a dimension is exceeded, report_usage stops the run.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "max_tokens": {
      "type": "integer"
    },
    "max_turns": {
      "type": "integer"
    },
    "max_usd_micros": {
      "description": "USD in micros ($1 = 1000000)",
      "type": "integer"
    },
    "max_wall_secs": {
      "description": "wall-clock budget vs the working clock",
      "type": "integer"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "max_tokens",
    "max_usd_micros",
    "max_turns",
    "max_wall_secs"
  ],
  "type": "object"
}
```

### `update_thread_budget`

Change only the budget dimensions you name, leaving the rest as they are. An omitted dimension is untouched; an explicit null clears that cap. Use this to raise or lower one limit without restating the others — set_thread_budget replaces the whole envelope. An unrecognized key is rejected rather than ignored.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "max_tokens": {
      "anyOf": [
        {
          "type": "integer"
        },
        {
          "type": "null"
        }
      ]
    },
    "max_turns": {
      "anyOf": [
        {
          "type": "integer"
        },
        {
          "type": "null"
        }
      ]
    },
    "max_usd_micros": {
      "anyOf": [
        {
          "type": "integer"
        },
        {
          "type": "null"
        }
      ],
      "description": "USD in micros ($1 = 1000000)"
    },
    "max_wall_secs": {
      "anyOf": [
        {
          "type": "integer"
        },
        {
          "type": "null"
        }
      ],
      "description": "wall-clock budget vs the working clock"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `get_thread_budget`

A thread's budget envelope with accumulated usage, or null if none is set.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `report_usage`

Record one retry-safe usage heartbeat for your active claim. Reuse usage_report_id only for an exact retry; claim_lease_id fences stale workers. Maidan derives reporter from auth and payer from the thread. Tiered tokens + the immutable price snapshot must calculate to usd_micros. A binding cap atomically stops the run.

**Capability:** `thread:transition`

```json
{
  "additionalProperties": false,
  "properties": {
    "claim_lease_id": {
      "description": "active claim fencing token",
      "format": "uuid",
      "type": "string"
    },
    "model": {
      "maxLength": 255,
      "minLength": 1,
      "type": "string"
    },
    "price_snapshot": {
      "additionalProperties": false,
      "properties": {
        "cache_read_usd_micros_per_million": {
          "minimum": 0,
          "type": "integer"
        },
        "cache_write_usd_micros_per_million": {
          "minimum": 0,
          "type": "integer"
        },
        "input_usd_micros_per_million": {
          "minimum": 0,
          "type": "integer"
        },
        "output_usd_micros_per_million": {
          "minimum": 0,
          "type": "integer"
        }
      },
      "required": [
        "input_usd_micros_per_million",
        "output_usd_micros_per_million",
        "cache_read_usd_micros_per_million",
        "cache_write_usd_micros_per_million"
      ],
      "type": "object"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    },
    "tokens": {
      "additionalProperties": false,
      "properties": {
        "cache_read": {
          "minimum": 0,
          "type": "integer"
        },
        "cache_write": {
          "minimum": 0,
          "type": "integer"
        },
        "input": {
          "minimum": 0,
          "type": "integer"
        },
        "output": {
          "minimum": 0,
          "type": "integer"
        }
      },
      "required": [
        "input",
        "output",
        "cache_read",
        "cache_write"
      ],
      "type": "object"
    },
    "turns": {
      "default": 0,
      "type": "integer"
    },
    "usage_report_id": {
      "description": "globally unique idempotency key",
      "format": "uuid",
      "type": "string"
    },
    "usd_micros": {
      "description": "validated USD charge in micros ($1 = 1000000)",
      "minimum": 0,
      "type": "integer"
    }
  },
  "required": [
    "thread_id",
    "usage_report_id",
    "claim_lease_id",
    "model",
    "tokens",
    "usd_micros",
    "price_snapshot"
  ],
  "type": "object"
}
```

### `list_dlq`

A channel's agent-work dead-letter queue — runs stopped for exceeding their budget, newest first. Triage these (retry, raise the budget, give up).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "limit": {
      "default": 50,
      "description": "clamped 1..=200",
      "type": "integer"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `get_tool_transcript`

A thread's tool-call transcript: every ToolUse block correlated with its ToolResult by id. A token-lean projection that drops text/code blocks and bodies.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "limit": {
      "default": 200,
      "description": "max messages to scan",
      "maximum": 500,
      "minimum": 1,
      "type": "integer"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `assign_thread`

Assign or hand off a thread/task to a member, optionally with a handoff note delivered to subscribers on the assignment event.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "assignee_id": {
      "description": "member to assign the thread to",
      "format": "uuid",
      "type": "string"
    },
    "note": {
      "description": "optional handoff note for the assignee",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "assignee_id"
  ],
  "type": "object"
}
```

### `claim_thread`

Atomically claim an unassigned thread for a member. Returns {thread, claimed}; claimed=false if it was already assigned.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `unassign_thread`

Clear a thread's assignee.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `transition_thread`

Advance a thread's FSM state (start_review, close, or archive). The MCP twin of REST POST /threads/:id. Separation of duties, the required-reviewers close-gate, and unresolved refutes all apply identically — there is no MCP bypass. Returns the updated thread.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "action": {
      "description": "start_review, close, or archive",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "action"
  ],
  "type": "object"
}
```

### `list_assigned_threads`

List the threads currently assigned to a member (their work queue), oldest first.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `set_wait`

Set (upsert) a wait timer on a thread (G2): it is waiting until wait_until, and on timeout the sweeper escalates via on_timeout — never a decision (notify reaches the owner; park also marks the thread unclaimable). Default policy is notify. Cancel it when the awaited thing happens. Requires thread:transition.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "on_timeout": {
      "default": "notify",
      "description": "escalation policy on timeout",
      "enum": [
        "notify",
        "park"
      ],
      "type": "string"
    },
    "reason": {
      "description": "why the thread is waiting",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    },
    "wait_until": {
      "description": "deadline (RFC 3339)",
      "format": "date-time",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "wait_until"
  ],
  "type": "object"
}
```

### `cancel_wait`

Cancel a thread's wait — the awaited thing happened (G2). {cancelled} is false when no wait was set. Requires thread:transition.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `get_wait`

The thread's wait timer (deadline, on_timeout policy, reason, fired_at), or null if none is set (G2).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `set_priority`

Set (upsert) a thread's dispatch priority (G3 fair dispatch). Higher = more urgent (default 0). claim_next orders by an effective rank = this priority aged up the longer the thread waits, so priority jumps the queue without starving long-waiting tasks. Requires thread:transition.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "priority": {
      "description": "higher = more urgent; default 0",
      "type": "integer"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "priority"
  ],
  "type": "object"
}
```

### `get_priority`

The thread's dispatch-priority record, or null (which means the default priority 0) (G3).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `mark_unclaimable`

Park a thread from dispatch (G3): claim_next skips it and an explicit claim is refused, until cleared. An explicit park (needs triage, waiting on external, broken) — distinct from blocked-by-deps / blocked-by-gate / skill-miss. Reason must be non-empty. Requires thread:transition.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "reason": {
      "description": "why the thread is parked",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "reason"
  ],
  "type": "object"
}
```

### `mark_claimable`

Un-park a thread (G3) — it becomes claimable again. {cleared} is false when it was not parked. Requires thread:transition.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `list_unclaimable`

The parked (unclaimable) threads in a channel (G3), newest first — for triage.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `set_thread_block`

Set (upsert) an explicit dispatch block on a thread (G14): claim_next skips it and an explicit claim is refused, until cleared. reason is the closed enum dag|gate|human|child|quota|unclaimable — not a free string. Distinct from DAG-children-must-be-terminal. Requires thread:transition.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "reason": {
      "enum": [
        "dag",
        "gate",
        "human",
        "child",
        "quota",
        "unclaimable"
      ],
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "reason"
  ],
  "type": "object"
}
```

### `get_thread_block`

The thread's explicit dispatch block, or null when unblocked (G14). Requires workspace:read.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `clear_thread_block`

Clear an explicit dispatch block (G14). Emits BlockedResolved so waiters can observe the unblock. {cleared} is false when it was not blocked. Requires thread:transition.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `list_blocked_threads`

The explicitly blocked threads in a channel (G14), newest first — for triage. Distinct from queue-depth blocked (unfinished DAG deps).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `set_wip_limit`

Set or clear this workspace's WIP limit (G11): the max concurrent live claims any one member may hold. limit >= 0 caps it (0 freezes claiming); omit or null clears it (unlimited). Applies to your own workspace. Requires workspace:write.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "limit": {
      "anyOf": [
        {
          "minimum": 0,
          "type": "integer"
        },
        {
          "type": "null"
        }
      ],
      "description": "max concurrent live claims per member; null/omit = unlimited"
    }
  },
  "type": "object"
}
```

### `get_wip_limit`

This workspace's WIP limit (max concurrent live claims per member), or null when unset (unlimited).

**Capability:** `workspace:read`

```json
{
  "properties": {},
  "type": "object"
}
```

### `set_spawn_budget`

Set this workspace's spawn budget (G6): how far an agent family may fan out. max_children caps the direct child threads per parent, max_depth the thread nesting, max_tools the tool calls recorded on one thread. A full replace — an omitted or null axis is unlimited, so calling with no arguments clears the budget; 0 freezes an axis. Keep the caps small: coordination cost grows quadratically in the number of agents. Requires workspace:write.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "max_children": {
      "anyOf": [
        {
          "minimum": 0,
          "type": "integer"
        },
        {
          "type": "null"
        }
      ],
      "description": "max direct child threads per parent; null = unlimited"
    },
    "max_depth": {
      "anyOf": [
        {
          "minimum": 0,
          "type": "integer"
        },
        {
          "type": "null"
        }
      ],
      "description": "max thread nesting depth (a root thread is depth 1); null = unlimited"
    },
    "max_tools": {
      "anyOf": [
        {
          "minimum": 0,
          "type": "integer"
        },
        {
          "type": "null"
        }
      ],
      "description": "max tool calls recorded on one thread; null = unlimited"
    }
  },
  "type": "object"
}
```

### `get_spawn_budget`

This workspace's spawn budget as {max_children, max_depth, max_tools}; a null axis is unlimited. Read it before spawning helpers to see how much fan-out is left.

**Capability:** `workspace:read`

```json
{
  "properties": {},
  "type": "object"
}
```

### `get_member_wip`

A member's current live-claim count against the workspace WIP limit ({live_claims, limit}) — for backpressure decisions before claiming more work.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `claim_next_thread`

Atomically claim the oldest claimable thread in a channel for a member (claimable = unassigned or its lease expired). Returns the claimed thread with a content-addressed pin {uri, content_hash}, or null when there is no claimable work.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "lease_secs": {
      "description": "optional lease deadline in seconds; the claim is reclaimable after it lapses (omit for a durable claim)",
      "type": "integer"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `renew_claim`

Extend a claimed thread's lease (heartbeat). Only the current assignee holding the matching fencing token may renew; a stale holder whose claim was reclaimed is rejected.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "claim_lease_id": {
      "description": "the fencing token from the claim response's thread.claim_lease_id",
      "format": "uuid",
      "type": "string"
    },
    "lease_secs": {
      "description": "new lease deadline in seconds from now",
      "type": "integer"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "claim_lease_id",
    "lease_secs"
  ],
  "type": "object"
}
```

### `acknowledge_claim`

Acknowledge a claimed thread and start its working clock (work_started_at): the current holder signals it has begun work, distinct from just holding the claim. Only the assignee holding the matching fencing token may acknowledge; idempotent (the first start time is kept).

**Capability:** `thread:transition`

```json
{
  "properties": {
    "claim_lease_id": {
      "description": "the fencing token from the claim response's thread.claim_lease_id",
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "claim_lease_id"
  ],
  "type": "object"
}
```

### `release_claim`

Release a claim (graceful handoff): the current holder returns the thread to the queue immediately by presenting its fencing token, instead of letting the lease lapse — e.g. an agent shutting down cleanly. Only the assignee holding the matching token may release. Clears the assignment and working clock and emits thread_assignment_changed.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "claim_lease_id": {
      "description": "the fencing token from the claim response's thread.claim_lease_id",
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "claim_lease_id"
  ],
  "type": "object"
}
```

### `add_thread_dependency`

Add a task-dependency edge: the thread depends on depends_on_thread_id and stays blocked (won't be handed out by claim_next) until that dependency reaches a terminal state. Both threads must be in the same workspace.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "depends_on_thread_id": {
      "description": "the task it depends on",
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "description": "the dependent task",
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "depends_on_thread_id"
  ],
  "type": "object"
}
```

### `list_thread_dependencies`

List a task's dependencies plus whether it is ready to run (true when every dependency is terminal).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `get_queue_depth`

A channel's task-queue depth: counts of its open task threads as {open, ready, assigned, blocked}, for deciding whether to scale workers. ready is what claim_next_thread could take now.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `get_channel_occupancy`

A channel's occupancy as {open, queued, claimed, working, blocked}: the two-clocks refinement of get_queue_depth. It splits held work into claimed (an agent grabbed the task but hasn't acknowledged it via acknowledge_claim) and working (acknowledged and underway) — surfacing a claimed-but-idle agent. queued/blocked mirror get_queue_depth's ready/blocked.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `set_thread_lineage`

Home a producer's run_id on a thread as parent_run_id. Accepts the producer's string as-is (does not mint a parallel id). Empty / whitespace / over-long is rejected. Use when attributing nested work to a producer run; set_thread_result also auto-homes when the payload carries run_id.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "parent_run_id": {
      "description": "the producer's run identifier (e.g. waiter envelope run_id)",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "parent_run_id"
  ],
  "type": "object"
}
```

### `get_thread_lineage`

Read a thread's run lineage (parent_run_id + set_at), or null if none has been set.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `list_run_threads`

List threads in the caller's workspace that share a producer parent_run_id, oldest first. Nested children given the same value are included. Private-channel rows the caller cannot access are omitted. F7 mute is not consulted.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "parent_run_id": {
      "description": "the producer's run identifier",
      "type": "string"
    }
  },
  "required": [
    "parent_run_id"
  ],
  "type": "object"
}
```

### `get_run_occupancy`

Nested occupancy for a producer run as {parent_run_id, open, queued, claimed, working, blocked}: the two-clocks partition of every open workspace thread that shares parent_run_id. F7 mute is orthogonal (a muted nested thread still counts). Unknown / unused run returns zeros.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "parent_run_id": {
      "description": "the producer's run identifier",
      "type": "string"
    }
  },
  "required": [
    "parent_run_id"
  ],
  "type": "object"
}
```

### `set_thread_result`

Attach a task's structured result (arbitrary JSON). Upserts one result per thread and notifies waiters via a thread_result_set event. Use when finishing a task so a requester or parent can read the output.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "result": {
      "description": "structured JSON result payload (an object)",
      "type": "object"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "result"
  ],
  "type": "object"
}
```

### `get_thread_result`

Read a task's structured result, or null if none has been produced yet.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `list_thread_results`

List thread results in the caller's workspace, newest first. Optional result_kind is an exact-match facet on the namespaced string (e.g. example.review.result/1), not a closed enum. Private-channel rows the caller cannot access are omitted.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "limit": {
      "default": 50,
      "maximum": 500,
      "minimum": 1,
      "type": "integer"
    },
    "result_kind": {
      "description": "exact namespaced result_kind (e.g. example.review.result/1); omit to list every accessible result",
      "type": "string"
    }
  },
  "type": "object"
}
```

### `list_result_deliveries`

List per-target delivery status for a thread's structured result (disposition, external reference, last error). Empty means the result was not routed anywhere, which is valid. workspace:read + thread access.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `replay_result_delivery`

Re-enqueue one result delivery onto the egress outbox. Re-checks the workspace allowlist (an unblessed target stays skipped). Does not bump armed_revision. workspace:write + thread access.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "delivery_id": {
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "delivery_id"
  ],
  "type": "object"
}
```

### `set_thread_owner`

Set (or clear, by omitting owner_id) a thread's durable owner — the accountable party, distinct from the assignee/claimer. Once an owner is set, the claimer can no longer land (close/archive) its own work; the owner or another member must (separation of duties).

**Capability:** `thread:transition`

```json
{
  "properties": {
    "owner_id": {
      "description": "the owner to set; omit to clear",
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `rename_thread`

Rename a thread — give a titled thread a new name (e.g. name a post-derived child thread). The title must not be blank. A rename is metadata, not activity, so it does not float the thread in the recent-activity order.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    },
    "title": {
      "description": "the new thread title",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "title"
  ],
  "type": "object"
}
```

### `set_thread_steer`

Set (upsert) a thread's persisted steer — durable steering guidance that survives claims and handoffs, so a resuming or newly-assigned agent reads the current steer. Latest wins.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "steer": {
      "description": "the steering instruction",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "steer"
  ],
  "type": "object"
}
```

### `get_thread_steer`

Read a thread's current steer, or null if none is set. A resuming or newly-assigned agent reads this to follow the current steering guidance.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `wait_for_result`

Block until a task's result is produced (a thread_result_set event for thread_id), returning the result payload, or null on timeout. The coordination wait for spawn/wait/aggregate. Pass since_log_id (your high-water log_id) to also catch a result set in the gap before this call subscribes; omit it for pure-live (read get_thread_result first for an already-produced result).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "since_log_id": {
      "description": "lookback anchor: replay the log for a matching event with log_id greater than this before parking live",
      "type": "integer"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    },
    "timeout_ms": {
      "description": "wait window ms (default 30000, clamped 1000-300000)",
      "type": "integer"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `get_dependency_results`

Gather the structured results of a parent task's dependencies as a list of {thread_id, result} objects (result null if not produced yet), skipping dependencies you can't access. The spawn/wait/aggregate read for a parent task.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "description": "the parent task",
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `add_member_skill`

Declare a skill (free-form tag) for a member. Skill routing gates claim_next: a task is claimable by a member only if it holds all the task's required skills.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "skill": {
      "type": "string"
    }
  },
  "required": [
    "member_id",
    "skill"
  ],
  "type": "object"
}
```

### `list_member_skills`

List a member's declared skills.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `add_thread_required_skill`

Add a required skill to a task. Only a member holding every required skill can claim the task via claim_next_thread.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "skill": {
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "skill"
  ],
  "type": "object"
}
```

### `list_thread_required_skills`

List a task's required skills.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `create_task_schedule`

Create a task schedule: when due, the sweeper creates a thread titled `title` in `channel_id` (or, when recipe_id is set, instantiates that recipe — parent + DAG children — instead). interval_secs omitted = one-shot; a positive value = recurring. first_run_at omitted = fire on the next tick.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "first_run_at": {
      "description": "when to first fire (default: now)",
      "format": "date-time",
      "type": "string"
    },
    "interval_secs": {
      "description": "recurrence period in seconds; omit for a one-shot",
      "type": "integer"
    },
    "recipe_id": {
      "description": "when set, each firing instantiates this recipe instead of a bare thread (skipped if the prior run is still in flight)",
      "format": "uuid",
      "type": "string"
    },
    "title": {
      "type": "string"
    }
  },
  "required": [
    "channel_id",
    "title"
  ],
  "type": "object"
}
```

### `list_task_schedules`

List the caller's workspace task schedules (filtered to channels the caller can access).

**Capability:** `workspace:read`

```json
{
  "properties": {},
  "type": "object"
}
```

### `create_recipe`

Create a recipe: a reusable thread-type blueprint. spec = {params, definition_of_done, retry, children}, where each child is {key, title, required_skills, depends_on (sibling keys)}. Instantiating it (instantiate_recipe) builds a parent thread + a child per child + wires the DAG + attaches skills. NOT a recipe VM — a blueprint the room instantiates.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "name": {
      "type": "string"
    },
    "spec": {
      "description": "the RecipeSpec (params, definition_of_done, retry, children)",
      "type": "object"
    }
  },
  "required": [
    "channel_id",
    "name",
    "spec"
  ],
  "type": "object"
}
```

### `list_recipes`

List the caller's workspace recipes (filtered to channels the caller can access).

**Capability:** `workspace:read`

```json
{
  "properties": {},
  "type": "object"
}
```

### `instantiate_recipe`

Instantiate a recipe into a parent thread + its DAG children (copy-on-fire: the recipe bytes are frozen into the run). params are validated against the recipe's declared params (required ones must be present). Returns the RecipeRun.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "params": {
      "description": "instantiation params (must satisfy the recipe's required params)",
      "type": "object"
    },
    "recipe_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "recipe_id"
  ],
  "type": "object"
}
```

### `list_secrets`

List the caller's workspace secrets (metadata only — id, name, timestamps; NEVER the value). Use resolve_secret to fetch a value at exec.

**Capability:** `secret:read`

```json
{
  "properties": {},
  "type": "object"
}
```

### `resolve_secret`

Resolve a named secret to its value (the 'fetch at exec' path). The value is decrypted server-side and returned only in this response — it never enters the event log. Returns null-name error if the secret is unknown.

**Capability:** `secret:read`

```json
{
  "properties": {
    "name": {
      "description": "the secret name (the part after secret://)",
      "type": "string"
    }
  },
  "required": [
    "name"
  ],
  "type": "object"
}
```

### `freeze_member`

Freeze a member (the kill-switch): drops their active leases (releases their claimed threads) and makes claim_next refuse them. Returns the freeze record + the count released. The member stays frozen until unfreeze_member. Requires token:admin. NOT a thread/workspace pause.

**Capability:** `token:admin`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "reason": {
      "description": "optional audit note",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `unfreeze_member`

Lift a member's freeze so they can claim work again. Requires token:admin. Returns {unfrozen} (false if they were not frozen).

**Capability:** `token:admin`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `list_frozen_members`

List the frozen members in the caller's workspace (member_id, frozen_at, frozen_by, reason). Requires token:admin.

**Capability:** `token:admin`

```json
{
  "properties": {},
  "type": "object"
}
```

### `create_share_ticket`

Issue a read-only cross-organization ticket for one channel and an explicit artifact allowlist. Ownership is bound to the authenticated member. Lifetime is capped at 48 hours; the secret is returned once and only its hash is stored. Requires token:admin.

**Capability:** `token:admin`

```json
{
  "properties": {
    "artifact_shas": {
      "items": {
        "pattern": "^[0-9a-f]{64}$",
        "type": "string"
      },
      "maxItems": 100,
      "type": "array"
    },
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "expires_at": {
      "format": "date-time",
      "type": "string"
    }
  },
  "required": [
    "channel_id",
    "expires_at"
  ],
  "type": "object"
}
```

### `list_share_tickets`

List share tickets and their explicit artifact scopes in the caller's workspace. Secrets are never returned after creation. Requires token:admin.

**Capability:** `token:admin`

```json
{
  "properties": {},
  "type": "object"
}
```

### `revoke_share_ticket`

Immediately revoke a share ticket in the caller's workspace. Requires token:admin.

**Capability:** `token:admin`

```json
{
  "properties": {
    "ticket_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "ticket_id"
  ],
  "type": "object"
}
```

### `export_workspace`

Export a workspace as a signed maidan.workspace.export/1 envelope. Tokens die on export: API tokens and secrets are omitted. A blank instance can verify the file without calling this host. Requires token:admin and MAIDAN_EXPORT_SIGNING_KEY.

**Capability:** `token:admin`

```json
{
  "properties": {
    "workspace_id": {
      "description": "defaults to the caller's workspace",
      "format": "uuid",
      "type": "string"
    }
  },
  "type": "object"
}
```

### `verify_workspace_export`

Verify a signed workspace export without importing it. Fail-closed on tamper, a bad signature, stuffed secret fields, or a public key outside MAIDAN_EXPORT_VERIFY_KEYS when that pin is set. An empty pin checks integrity against the embedded key only. Requires token:admin.

**Capability:** `token:admin`

```json
{
  "properties": {
    "envelope": {
      "description": "the signed envelope; you may also pass the envelope fields at the top level",
      "type": "object"
    }
  },
  "type": "object"
}
```

### `import_workspace`

Verify then import a signed workspace export. mode new remaps ids into a fresh workspace; restore keeps original ids and fails if that workspace exists unless force is true. Tokens die on export: mint new tokens after import. Requires token:admin.

**Capability:** `token:admin`

```json
{
  "properties": {
    "envelope": {
      "description": "the signed maidan.workspace.export/1 envelope",
      "type": "object"
    },
    "force": {
      "description": "erase an existing workspace when mode is restore",
      "type": "boolean"
    },
    "mode": {
      "description": "defaults to new",
      "enum": [
        "new",
        "restore"
      ],
      "type": "string"
    }
  },
  "required": [
    "envelope"
  ],
  "type": "object"
}
```

### `get_log_snapshot`

Hashed event-log snapshot for this workspace (getRepo-shaped, not MST/CAR). Header plus graph_hash is workspace:read. Pass include_graph true for the domain graph; that requires token:admin. Complements hash-chain verify of the retained suffix: this covers a pruned prefix so a peer can resume without trusting the host for history it never saw.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "include_graph": {
      "description": "include the domain graph; requires token:admin (default false)",
      "type": "boolean"
    },
    "workspace_id": {
      "description": "defaults to the caller's workspace",
      "format": "uuid",
      "type": "string"
    }
  },
  "type": "object"
}
```

### `catch_up_events`

Since-LSN catch-up page after a snapshot (or a prior page). Events have id greater than after_lsn, hash-chain checked from the predecessor. A pruned-gap cursor fails closed and names the snapshot path to refetch; a broken chain fails closed. Requires workspace:read.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "after_lsn": {
      "description": "exclusive cursor; 0 starts from the retained floor",
      "type": "integer"
    },
    "limit": {
      "description": "page size, 1 to 500, default 100",
      "type": "integer"
    },
    "workspace_id": {
      "description": "defaults to the caller's workspace",
      "format": "uuid",
      "type": "string"
    }
  },
  "type": "object"
}
```

### `verify_event_chain`

Verify the retained event-log hash chain for a workspace. Returns the chain report when intact; fails closed on a splice or rewrite. Twin of GET /workspaces/{id}/events/verify. Requires workspace:read.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "workspace_id": {
      "description": "defaults to the caller's workspace",
      "format": "uuid",
      "type": "string"
    }
  },
  "type": "object"
}
```

### `list_tombstones`

Tombstone and deletion explorer for this workspace. Soft-deleted messages (body already cleared) plus, when include_purged is true, hard-purged reconstructions from MessageTombstoned events. Private-channel and DM rows the caller cannot access are omitted. Twin of GET /workspaces/{id}/tombstones. Requires workspace:read.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "description": "optional channel scope; gated when present",
      "format": "uuid",
      "type": "string"
    },
    "include_purged": {
      "description": "reconstruct hard-deleted rows from MessageTombstoned (default false)",
      "type": "boolean"
    },
    "limit": {
      "description": "page size, 1 to 500, default 100",
      "type": "integer"
    },
    "thread_id": {
      "description": "optional thread scope; gated when present",
      "format": "uuid",
      "type": "string"
    },
    "workspace_id": {
      "description": "defaults to the caller's workspace",
      "format": "uuid",
      "type": "string"
    }
  },
  "type": "object"
}
```

### `list_message_backlinks`

Incoming pointers at a message: RelationKind reverse edges plus pins, reactions, and votes. Mentions are outgoing and omitted. Works on a retained tombstone; fails not-found after hard purge. Twin of GET /messages/{id}/backlinks. Requires workspace:read.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "message_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "message_id"
  ],
  "type": "object"
}
```

### `get_kind_census`

EventKind counts for a workspace, optionally narrowed to a channel or thread. Inaccessible private channels are excluded from the totals. Twin of GET /workspaces/{id}/kind-census. Requires workspace:read.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "description": "optional channel scope; gated when present",
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "description": "optional thread scope; gated when present",
      "format": "uuid",
      "type": "string"
    },
    "workspace_id": {
      "description": "defaults to the caller's workspace",
      "format": "uuid",
      "type": "string"
    }
  },
  "type": "object"
}
```

### `create_memory_block`

Create a labeled memory block — a Letta-shaped shared object {label, description, limit, read_only, value} in the workspace that a thread can attach to (a room object). It is how a parent watches a child's result block without a nested runtime: not a transcript, not RAG. Concurrent-safe on the label (re-creating a label returns the existing block). The caller owns it.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "char_limit": {
      "description": "optional max value length in characters",
      "type": "integer"
    },
    "description": {
      "description": "what the block is for",
      "type": "string"
    },
    "label": {
      "description": "the block's within-workspace key",
      "type": "string"
    },
    "read_only": {
      "description": "refuse writes when true (default false)",
      "type": "boolean"
    },
    "value": {
      "description": "initial content (default empty)",
      "type": "string"
    }
  },
  "required": [
    "label"
  ],
  "type": "object"
}
```

### `get_memory_block`

Get a memory block by label within the caller's workspace, or null if none. This is the watch-a-child's-result-block read (poll it).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "label": {
      "type": "string"
    }
  },
  "required": [
    "label"
  ],
  "type": "object"
}
```

### `list_memory_blocks`

List the memory blocks in the caller's workspace (id, label, description, limit, read_only, value, owner).

**Capability:** `workspace:read`

```json
{
  "properties": {},
  "type": "object"
}
```

### `set_memory_block_value`

Full-rewrite a memory block's value by label (last-writer-wins). A read-only block or a value over the block's char limit is rejected. Use this to publish a result other threads watch.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "label": {
      "type": "string"
    },
    "value": {
      "type": "string"
    }
  },
  "required": [
    "label",
    "value"
  ],
  "type": "object"
}
```

### `attach_memory_block`

Attach a memory block (by label) to a thread so the thread carries it as a room object — how a parent shares a block with a child. Idempotent. Returns {attached}.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "label": {
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "label"
  ],
  "type": "object"
}
```

### `detach_memory_block`

Detach a memory block (by id) from a thread. Idempotent. Returns {detached} (false if it was not attached).

**Capability:** `workspace:write`

```json
{
  "properties": {
    "block_id": {
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "block_id"
  ],
  "type": "object"
}
```

### `list_thread_memory_blocks`

List the memory blocks attached to a thread (its room objects), by label.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `wait_for_memory_block`

Block until a memory block (by label) is rewritten in the caller's workspace, or the timeout lapses. Returns the block with its fresh value, or null on timeout. This is how a parent watches a child's result block without a nested runtime. Live: only sees updates after subscribing, so read the current value with get_memory_block first.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "label": {
      "type": "string"
    },
    "timeout_ms": {
      "description": "long-poll window (default 30000, clamped 1000-300000)",
      "type": "integer"
    }
  },
  "required": [
    "label"
  ],
  "type": "object"
}
```

### `set_review_requirement`

Set (upsert) a thread's required-reviewers gate (G5): required_count distinct qualifying approvals before it can close. An approval qualifies when the reviewer is neither the owner nor the assignee (separation of duties) and, when a named reviewer set exists, is in it. A refutes edge also blocks close. Requires thread:transition.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "required_count": {
      "description": "approvals needed (>= 0)",
      "type": "integer"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "required_count"
  ],
  "type": "object"
}
```

### `add_reviewer`

Name a reviewer for a thread (G5) — the eligible set. Empty set = open review (any qualifying member). Idempotent. Requires thread:transition.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "member_id"
  ],
  "type": "object"
}
```

### `submit_review`

Submit a review decision as the caller (G5): approve or request_changes. The reviewer is you; an owner/assignee may submit but it will not count toward the requirement (separation of duties). Re-submitting changes your decision. Requires thread:transition.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "decision": {
      "enum": [
        "approve",
        "request_changes"
      ],
      "type": "string"
    },
    "note": {
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "decision"
  ],
  "type": "object"
}
```

### `get_review_status`

Read a thread's review standing: required_count, approvals (distinct qualifying), and approvals_met. This is the approval side of the close-gate; a refutes edge is checked separately when closing.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `list_reviews`

List a thread's review decisions (reviewer, decision, note).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `set_land_gate`

Record a land-gate pointer on a thread: status pass or fail, optional artifact_sha, optional land green/amber/red. The room holds the pointer; an external verifier records pass/fail. A qualifying green pass (land-gate-skilled member who is not the implementer) is required to close once the gate is armed. Amber is flags-then-still-engages and is not a land. Requires thread:transition. The caller must have declared the land_gate skill.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "artifact_sha": {
      "type": "string"
    },
    "land": {
      "enum": [
        "green",
        "amber",
        "red"
      ],
      "type": "string"
    },
    "status": {
      "enum": [
        "pass",
        "fail"
      ],
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "status"
  ],
  "type": "object"
}
```

### `get_land_gate`

Read a thread's land-gate standing: required, pointer, land (green/amber/red), landable. No pointer is vacuous green. Requires workspace:read.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `require_land_gate`

Arm the land-gate close-gate on a thread without a pointer yet so closed refuses until a qualifying green pass arrives. Idempotent. Requires thread:transition.

**Capability:** `thread:transition`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `clear_land_gate`

Clear a thread's land-gate pointer and requirement. Requires thread:transition.

**Capability:** `channel:admin`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `set_glossary_term`

Define (or redefine) a term in the workspace's shared glossary — the canonical term -> definition so agents use words the same way (the anti-drift pin; the target of a `defines` reference). Upserts on the term.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "aliases": {
      "description": "alternate labels for the same term",
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "definition": {
      "type": "string"
    },
    "term": {
      "type": "string"
    }
  },
  "required": [
    "term",
    "definition"
  ],
  "type": "object"
}
```

### `get_glossary_term`

Look up one term's canonical definition in the workspace glossary. Returns null when the term is undefined.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "term": {
      "type": "string"
    }
  },
  "required": [
    "term"
  ],
  "type": "object"
}
```

### `list_glossary_terms`

List all defined terms in the workspace's shared glossary, ordered by term.

**Capability:** `workspace:read`

```json
{
  "properties": {},
  "type": "object"
}
```

### `wait_for_ready`

Block until a task becomes ready (its last blocking dependency reaches a terminal state, emitting thread_ready), or the timeout lapses. Returns the ThreadReady event, or null on timeout. Scoped to channel_id when given, else any accessible thread in the workspace. Pass since_log_id (your high-water log_id) to also catch readiness signalled in the gap before this call subscribes; omit it for pure-live (pick up already-ready work with claim_next_thread first).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "description": "optional: scope to one channel's tasks",
      "format": "uuid",
      "type": "string"
    },
    "since_log_id": {
      "description": "lookback anchor: replay the log for a matching event with log_id greater than this before parking live",
      "type": "integer"
    },
    "timeout_ms": {
      "default": 30000,
      "description": "long-poll window in milliseconds",
      "maximum": 300000,
      "minimum": 1,
      "type": "integer"
    }
  },
  "type": "object"
}
```

### `wait_for_claim_expired`

Block until a claim's lease lapses and its thread is reclaimed by the next agent (emitting claim_expired), or the timeout lapses. A supervisor's 'an agent died' signal — returns the ClaimExpired event (its member_id is the dead holder), or null on timeout. Scoped to channel_id when given, else any accessible thread in the workspace. Pass since_log_id (your high-water log_id) to also catch an expiry reclaimed in the gap before this call subscribes; omit it for pure-live. A lease that expires but is never reclaimed emits nothing (poll get_channel_occupancy for that).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "description": "optional: scope to one channel's tasks",
      "format": "uuid",
      "type": "string"
    },
    "since_log_id": {
      "description": "lookback anchor: replay the log for a matching event with log_id greater than this before parking live",
      "type": "integer"
    },
    "timeout_ms": {
      "default": 30000,
      "description": "long-poll window in milliseconds",
      "maximum": 300000,
      "minimum": 1,
      "type": "integer"
    }
  },
  "type": "object"
}
```

### `wait_for_landed`

Block until a thread's linked GitHub PR lands (is merged, emitting thread_landed), or the timeout lapses. Returns the ThreadLanded event (repo, pr_number, merged_by, merge_commit_sha, title), or null on timeout. Scoped to thread_id and/or channel_id when given, else any accessible land in the workspace. The room 'steals the landed fact' — it does NOT transition the thread's FSM. Pass since_log_id (your high-water log_id) to also catch a land emitted in the gap before this call subscribes; omit it for pure-live. Live-only; the GET /mcp/stream SSE transport (kinds=thread_landed) is the resumable alternative.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "description": "optional: scope to one channel's threads",
      "format": "uuid",
      "type": "string"
    },
    "since_log_id": {
      "description": "lookback anchor: replay the log for a matching event with log_id greater than this before parking live",
      "type": "integer"
    },
    "thread_id": {
      "description": "optional: wait for this thread's PR to land",
      "format": "uuid",
      "type": "string"
    },
    "timeout_ms": {
      "default": 30000,
      "description": "long-poll window in milliseconds",
      "maximum": 300000,
      "minimum": 1,
      "type": "integer"
    }
  },
  "type": "object"
}
```

### `list_mentions`

List recent @mentions of a member (most recent first).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "limit": {
      "description": "max results (default 50, max 500)",
      "type": "integer"
    },
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `get_inbox`

A member's mention inbox: recent mentions plus the read-cursor, so an agent can find what it hasn't seen.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "limit": {
      "description": "max mentions (default 50, max 500)",
      "type": "integer"
    },
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `mark_inbox_read`

Advance a member's inbox read-cursor through an instant (RFC 3339); returns the updated inbox.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "read_through": {
      "format": "date-time",
      "type": "string"
    }
  },
  "required": [
    "member_id",
    "read_through"
  ],
  "type": "object"
}
```

### `wait_for_mention`

Block until the member is next @mentioned, or the timeout lapses. Returns the mention event, or null on timeout. Pass since_log_id (your high-water log_id from the last drain) to also catch a mention recorded in the gap before this call subscribes; omit it for pure-live behaviour (drain existing ones with get_inbox first).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "since_log_id": {
      "description": "lookback anchor: replay the log for a matching event with log_id greater than this before parking live",
      "type": "integer"
    },
    "timeout_ms": {
      "default": 30000,
      "description": "long-poll window in milliseconds",
      "maximum": 300000,
      "minimum": 1,
      "type": "integer"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `list_notifications`

List a member's per-recipient notifications, newest first. Set unread_only to see just the unread ones. The durable inbox the notification router fills; drain it here, then wait_for_notification for new ones.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "limit": {
      "default": 50,
      "maximum": 500,
      "minimum": 1,
      "type": "integer"
    },
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "unread_only": {
      "default": false,
      "type": "boolean"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `get_waiting_inbox`

The waiting-on-you inbox: everything needing a member's attention — their assigned non-terminal threads, the workspace's pending approval gates, and their unread mentions — oldest-waiting first, each aged against sla_secs (default 86400 = 24h) with an overdue flag. One member's queue, not @everyone.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "sla_secs": {
      "description": "overdue threshold in seconds (default 86400)",
      "type": "integer"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `list_notifications_grouped`

A member's notifications collapsed into per-thread groups, newest-activity first — a busy thread shows as one group (with its count, unread_count, and latest notification) instead of flooding the flat list. limit bounds how many notifications are scanned.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "limit": {
      "default": 50,
      "maximum": 500,
      "minimum": 1,
      "type": "integer"
    },
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "unread_only": {
      "default": false,
      "type": "boolean"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `list_buried_decisions`

A member's buried decisions — task results (decisions) produced by someone else in a channel or thread the member follows, since a given instant (default 7 days ago), newest first. The decisions the digest surfaces, queryable directly.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "limit": {
      "default": 50,
      "maximum": 200,
      "minimum": 1,
      "type": "integer"
    },
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "since": {
      "description": "default 7 days ago",
      "format": "date-time",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `get_manager_digest`

Compose this member's unread followed-member lifecycle notifications since an instant (default 7 days ago) into per-channel result, gate, and stuck counts. This is a notification view, not analytics.

**Capability:** `workspace:read`

```json
{
  "additionalProperties": false,
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "since": {
      "description": "default 7 days ago",
      "format": "date-time",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `get_unread_count`

A member's unread-notification badge count.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `mark_notification_read`

Mark one of a member's notifications read (recipient-scoped; marked=false if the id isn't this member's).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "notification_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id",
    "notification_id"
  ],
  "type": "object"
}
```

### `snooze_notification`

Snooze one of a member's notifications until an RFC 3339 instant — it drops out of the inbox and unread badge until then, and resurfaces once the snooze lapses. Recipient-scoped (snoozed=false if the id isn't this member's).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "notification_id": {
      "format": "uuid",
      "type": "string"
    },
    "until": {
      "format": "date-time",
      "type": "string"
    }
  },
  "required": [
    "member_id",
    "notification_id",
    "until"
  ],
  "type": "object"
}
```

### `wait_for_notification`

Block until the member gets a new notification-worthy event (today: mentions), or the timeout lapses. The general form of wait_for_mention. Returns the triggering event, or null on timeout. Pass since_log_id (your high-water log_id from the last drain) to also catch an event from the gap before this call subscribes; omit it for pure-live behaviour (drain with list_notifications first).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "since_log_id": {
      "description": "lookback anchor: replay the log for a matching event with log_id greater than this before parking live",
      "type": "integer"
    },
    "timeout_ms": {
      "default": 30000,
      "description": "long-poll window in milliseconds",
      "maximum": 300000,
      "minimum": 1,
      "type": "integer"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `set_notification_pref`

Set a member's mute preference for an event kind (kind is snake_case, e.g. mention_recorded). When muted, the router stops writing notifications of that kind for this member.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "kind": {
      "description": "event kind, snake_case",
      "type": "string"
    },
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "muted": {
      "type": "boolean"
    }
  },
  "required": [
    "member_id",
    "kind",
    "muted"
  ],
  "type": "object"
}
```

### `list_notification_prefs`

List a member's notification preferences (per-kind mute flags).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `set_delivery_mode`

Set a member's email delivery mode: immediate (a per-notification email) or digest (a periodic rollup instead). The two are mutually exclusive.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "mode": {
      "enum": [
        "immediate",
        "digest"
      ],
      "type": "string"
    }
  },
  "required": [
    "member_id",
    "mode"
  ],
  "type": "object"
}
```

### `get_delivery_mode`

Get a member's email delivery mode (immediate when never set).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `set_member_email`

Set a member's delivery email address (where their email notifications go). A light @ check; full validation happens at send.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "email": {
      "type": "string"
    },
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id",
    "email"
  ],
  "type": "object"
}
```

### `get_member_email`

Get a member's delivery email address (null when unset).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `delete_member_email`

Clear a member's delivery email address (opt out of email). Returns {deleted}.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `follow_channel`

Follow a channel so the member is notified of new messages there even without a mention (honors mutes). Requires access to the channel.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id",
    "channel_id"
  ],
  "type": "object"
}
```

### `unfollow_channel`

Stop following a channel (removed=false if not following).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id",
    "channel_id"
  ],
  "type": "object"
}
```

### `list_channel_follows`

List the channels a member follows.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `follow_thread`

Follow a thread so the member is notified of new messages in it even without a mention (honors mutes). Requires access to the thread.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id",
    "thread_id"
  ],
  "type": "object"
}
```

### `unfollow_thread`

Stop following a thread (removed=false if not following).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id",
    "thread_id"
  ],
  "type": "object"
}
```

### `list_thread_follows`

List the threads a member follows.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `follow_member`

Follow another same-workspace member's work occupancy. Self-follow is rejected.

**Capability:** `workspace:read`

```json
{
  "additionalProperties": false,
  "properties": {
    "followed_member_id": {
      "format": "uuid",
      "type": "string"
    },
    "member_id": {
      "description": "the follower",
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id",
    "followed_member_id"
  ],
  "type": "object"
}
```

### `get_member_occupancy`

Get a member's live occupancy: ephemeral presence plus assigned non-terminal threads visible to the caller.

**Capability:** `workspace:read`

```json
{
  "additionalProperties": false,
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `unfollow_member`

Stop following another member's work occupancy (removed=false if not following).

**Capability:** `workspace:read`

```json
{
  "additionalProperties": false,
  "properties": {
    "followed_member_id": {
      "format": "uuid",
      "type": "string"
    },
    "member_id": {
      "description": "the follower",
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id",
    "followed_member_id"
  ],
  "type": "object"
}
```

### `list_member_follows`

List the member-occupancy subscriptions owned by a member.

**Capability:** `workspace:read`

```json
{
  "additionalProperties": false,
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "member_id"
  ],
  "type": "object"
}
```

### `list_messages`

List messages in a thread.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "limit": {
      "default": 100,
      "maximum": 500,
      "minimum": 1,
      "type": "integer"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `post_message`

Post a message to a thread as the authenticated member.

**Capability:** `message:post`

```json
{
  "properties": {
    "body": {
      "description": "plain text; omit when sending typed content (body is derived from it)",
      "type": "string"
    },
    "content": {
      "description": "typed content blocks: {type: text|code|tool_use|tool_result|resource_link, ...}",
      "items": {
        "type": "object"
      },
      "type": "array"
    },
    "metadata": {
      "type": "object"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "body"
  ],
  "type": "object"
}
```

### `seed_from_message`

Seed a new titled work thread from a source message (the write side of 're-ask'), linked by a seeded_from reference edge. inclusion: 'pointer' (default, edge only) or 'quote' (a first message quoting the source). The source is untouched; N seeds per source. Lineage is queryable via list_references (dst=the source, relation=seeded_from).

**Capability:** `workspace:write`

```json
{
  "properties": {
    "channel_id": {
      "description": "target channel (default: the source's channel)",
      "format": "uuid",
      "type": "string"
    },
    "inclusion": {
      "default": "pointer",
      "enum": [
        "pointer",
        "quote"
      ],
      "type": "string"
    },
    "message_id": {
      "description": "the source message",
      "format": "uuid",
      "type": "string"
    },
    "title": {
      "type": "string"
    }
  },
  "required": [
    "message_id",
    "title"
  ],
  "type": "object"
}
```

### `edit_message`

Edit your own message (message:post). Only the author can edit a message; another member's message can be tombstoned, not rewritten.

**Capability:** `message:post`

```json
{
  "properties": {
    "body": {
      "description": "plain text; omit when sending typed content (body is derived from it)",
      "type": "string"
    },
    "content": {
      "description": "typed content blocks: {type: text|code|tool_use|tool_result|resource_link, ...}",
      "items": {
        "type": "object"
      },
      "type": "array"
    },
    "message_id": {
      "format": "uuid",
      "type": "string"
    },
    "metadata": {
      "type": "object"
    }
  },
  "required": [
    "message_id",
    "body"
  ],
  "type": "object"
}
```

### `record_mention`

Mark a member as mentioned in a message.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "message_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "message_id",
    "member_id"
  ],
  "type": "object"
}
```

### `cast_vote`

Cast a vote on a message (e.g. approve, request-changes, emoji). Optional confidence (0..1) for weighted consensus; re-casting the same kind updates your confidence.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "confidence": {
      "description": "optional confidence weight for weighted consensus",
      "maximum": 1,
      "minimum": 0,
      "type": "number"
    },
    "kind": {
      "type": "string"
    },
    "message_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "message_id",
    "kind"
  ],
  "type": "object"
}
```

### `add_reaction`

Add an emoji reaction to a message.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "emoji": {
      "type": "string"
    },
    "message_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "message_id",
    "emoji"
  ],
  "type": "object"
}
```

### `remove_reaction`

Remove an emoji reaction from a message.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "emoji": {
      "type": "string"
    },
    "message_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "message_id",
    "emoji"
  ],
  "type": "object"
}
```

### `list_reactions`

List emoji reactions on a message.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "message_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "message_id"
  ],
  "type": "object"
}
```

### `pin_message`

Pin a message to a thread.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "message_id": {
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "message_id"
  ],
  "type": "object"
}
```

### `unpin_message`

Unpin a message from a thread.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "message_id": {
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "message_id"
  ],
  "type": "object"
}
```

### `list_pins`

List pinned messages in a thread.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `add_reference`

Add a typed reference between two threads or messages.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "dst_id": {
      "format": "uuid",
      "type": "string"
    },
    "dst_kind": {
      "enum": [
        "thread",
        "message"
      ],
      "type": "string"
    },
    "relation": {
      "description": "typed relation; controlled set: supports/refutes/defines/depends/duplicates/grounds/supersedes (other values are allowed and round-trip verbatim)",
      "type": "string"
    },
    "src_id": {
      "format": "uuid",
      "type": "string"
    },
    "src_kind": {
      "enum": [
        "thread",
        "message"
      ],
      "type": "string"
    }
  },
  "required": [
    "src_kind",
    "src_id",
    "dst_kind",
    "dst_id",
    "relation"
  ],
  "type": "object"
}
```

### `list_references`

List references FROM a source (forward) or TO a target (reverse — 'what references this'), optionally filtered by relation. Provide exactly one of the src_kind+src_id or dst_kind+dst_id pair.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "dst_id": {
      "format": "uuid",
      "type": "string"
    },
    "dst_kind": {
      "enum": [
        "thread",
        "message"
      ],
      "type": "string"
    },
    "relation": {
      "description": "optional relation filter (controlled set: supports/refutes/defines/depends/duplicates/grounds/supersedes, or any custom value)",
      "type": "string"
    },
    "src_id": {
      "format": "uuid",
      "type": "string"
    },
    "src_kind": {
      "enum": [
        "thread",
        "message"
      ],
      "type": "string"
    }
  },
  "type": "object"
}
```

### `upload_artifact`

Store bytes in the artifact substrate and register metadata.

**Capability:** `artifact:upload`

```json
{
  "properties": {
    "content_base64": {
      "type": "string"
    },
    "kind": {
      "enum": [
        "screenshot",
        "recording",
        "transcript",
        "code_dump",
        "attachment"
      ],
      "type": "string"
    },
    "mime_type": {
      "type": "string"
    }
  },
  "required": [
    "kind",
    "content_base64"
  ],
  "type": "object"
}
```

### `begin_artifact_multipart`

Start an S3 multipart upload for a large artifact (requires S3 backend).

**Capability:** `artifact:upload`

```json
{
  "properties": {},
  "type": "object"
}
```

### `upload_artifact_multipart_part`

Upload one part of an in-progress multipart artifact.

**Capability:** `artifact:upload`

```json
{
  "properties": {
    "content_base64": {
      "type": "string"
    },
    "object_key": {
      "type": "string"
    },
    "part_number": {
      "minimum": 1,
      "type": "integer"
    },
    "upload_id": {
      "type": "string"
    }
  },
  "required": [
    "upload_id",
    "object_key",
    "part_number",
    "content_base64"
  ],
  "type": "object"
}
```

### `complete_artifact_multipart`

Finish multipart upload, content-address bytes, and register artifact metadata.

**Capability:** `artifact:upload`

```json
{
  "properties": {
    "kind": {
      "enum": [
        "screenshot",
        "recording",
        "transcript",
        "code_dump",
        "attachment"
      ],
      "type": "string"
    },
    "mime_type": {
      "type": "string"
    },
    "object_key": {
      "type": "string"
    },
    "parts": {
      "items": {
        "properties": {
          "etag": {
            "type": "string"
          },
          "part_number": {
            "type": "integer"
          }
        },
        "required": [
          "part_number",
          "etag"
        ],
        "type": "object"
      },
      "type": "array"
    },
    "upload_id": {
      "type": "string"
    }
  },
  "required": [
    "upload_id",
    "object_key",
    "parts",
    "kind"
  ],
  "type": "object"
}
```

### `abort_artifact_multipart`

Abort a failed multipart upload.

**Capability:** `artifact:upload`

```json
{
  "properties": {
    "object_key": {
      "type": "string"
    },
    "upload_id": {
      "type": "string"
    }
  },
  "required": [
    "upload_id",
    "object_key"
  ],
  "type": "object"
}
```

### `get_artifact_metadata`

Fetch artifact metadata by sha256 hex digest.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "sha256": {
      "maxLength": 64,
      "minLength": 64,
      "type": "string"
    }
  },
  "required": [
    "sha256"
  ],
  "type": "object"
}
```

### `search_messages`

Full-text, semantic, or hybrid search over a workspace's messages. Returns ranked hits with highlighted snippets.

**Capability:** `search:query`

```json
{
  "properties": {
    "after": {
      "description": "Only messages posted at/after this RFC 3339 instant (inclusive).",
      "format": "date-time",
      "type": "string"
    },
    "author_id": {
      "format": "uuid",
      "type": "string"
    },
    "before": {
      "description": "Only messages posted before this RFC 3339 instant (exclusive) — a half-open window with after.",
      "format": "date-time",
      "type": "string"
    },
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "embedding_model": {
      "description": "Semantic/hybrid only: registered model name (default: active provider).",
      "type": "string"
    },
    "hybrid_weight": {
      "description": "Hybrid only: semantic weight in [0,1] (default 0.5). combined = w*semantic + (1-w)*lexical over normalized scores.",
      "type": "number"
    },
    "kind": {
      "enum": [
        "human",
        "agent"
      ],
      "type": "string"
    },
    "limit": {
      "default": 25,
      "maximum": 500,
      "minimum": 1,
      "type": "integer"
    },
    "mode": {
      "default": "lexical",
      "enum": [
        "lexical",
        "semantic",
        "hybrid"
      ],
      "type": "string"
    },
    "query": {
      "minLength": 1,
      "type": "string"
    },
    "snippet_only": {
      "default": false,
      "description": "Drop full message body from each hit (keep only the snippet) to save tokens.",
      "type": "boolean"
    },
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id",
    "query"
  ],
  "type": "object"
}
```

### `register_slash_command`

Register a workspace slash command handler (http URL or MCP tool name).

**Capability:** `workspace:write`

```json
{
  "properties": {
    "description": {
      "type": "string"
    },
    "handler_kind": {
      "enum": [
        "http",
        "mcp_tool"
      ],
      "type": "string"
    },
    "handler_target": {
      "type": "string"
    },
    "name": {
      "type": "string"
    },
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id",
    "name",
    "handler_kind",
    "handler_target"
  ],
  "type": "object"
}
```

### `list_slash_commands`

List registered slash commands in a workspace.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id"
  ],
  "type": "object"
}
```

### `register_fsm_hook`

Register an FSM hook invoked on matching thread state transitions.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "from_state": {
      "enum": [
        "open",
        "in_review",
        "closed",
        "archived"
      ],
      "type": "string"
    },
    "handler_kind": {
      "enum": [
        "http",
        "mcp_tool"
      ],
      "type": "string"
    },
    "handler_target": {
      "type": "string"
    },
    "label": {
      "type": "string"
    },
    "to_state": {
      "enum": [
        "open",
        "in_review",
        "closed",
        "archived"
      ],
      "type": "string"
    },
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id",
    "handler_kind",
    "handler_target"
  ],
  "type": "object"
}
```

### `list_fsm_hooks`

List registered FSM automation hooks in a workspace.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id"
  ],
  "type": "object"
}
```

### `get_thread_context`

Pack thread messages, edits, references, FSM history, and the workspace glossary for agent prompts. Edits are lean by default (id/editor/timestamp only); pass include_edits=true for full before/after bodies. The glossary (canonical term definitions) is included by default when non-empty; pass include_glossary=false to drop it. Pass as_of=<event_id> to replay the thread as it stood at that event-log id (deterministic over the immutable log; audit / re-ask from before a tangent). Pass token_budget=<n> to cap the message page by estimated tokens: the opening message and the recent tail are kept, the middle is folded into an auditable 'elision' marker (Lost-in-the-Middle).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "as_of": {
      "description": "Event-log id: reconstruct the thread as it stood at that point (as-of replay). Omit for the live pack.",
      "type": "integer"
    },
    "include_accepted_decisions": {
      "default": true,
      "description": "Attach in-channel accepted/closed decisions (token-lean teasers) so a fresh claimer sees what the channel already decided. Waiter envelopes appear only when status is reviewed; result_kind is a namespaced string (e.g. example.review.result/1), not a closed enum. Withheld on DM channels. Set false for the leanest pack.",
      "type": "boolean"
    },
    "include_edits": {
      "default": false,
      "description": "Include full body_before/body_after on each edit (heavy); default returns edit metadata only.",
      "type": "boolean"
    },
    "include_glossary": {
      "default": true,
      "description": "Include the workspace glossary (grounding); omitted when empty. Set false for a token-tight pack.",
      "type": "boolean"
    },
    "include_parent_grounding": {
      "default": true,
      "description": "For a child thread, attach parent grounding (the parent's opening ask + latest decision) so a fresh claimer knows why the thread exists. Absent for root threads / cross-channel / DM parents. Set false for the leanest pack.",
      "type": "boolean"
    },
    "message_limit": {
      "maximum": 500,
      "minimum": 1,
      "type": "integer"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    },
    "token_budget": {
      "description": "Cap the message page by estimated tokens (chars/4): keep the opening message and the recent tail, fold the middle into an auditable 'elision' marker. Omit to cap by rows only.",
      "minimum": 1,
      "type": "integer"
    },
    "transition_limit": {
      "maximum": 200,
      "minimum": 1,
      "type": "integer"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `snapshot_thread_context`

Freeze the assembled context pack (live or as_of) into the content-addressed artifact store — a tamper-evident, deduped record of exactly what the agent was handed. Same params as get_thread_context; returns the artifact (kind=context_snapshot). Requires artifact:upload. Fetch the bytes via the artifact sha.

**Capability:** `artifact:upload`

```json
{
  "properties": {
    "as_of": {
      "description": "Event-log id: freeze the thread as it stood at that point. Omit for the live pack.",
      "type": "integer"
    },
    "include_accepted_decisions": {
      "default": true,
      "description": "Attach in-channel accepted decisions before freezing (see get_thread_context).",
      "type": "boolean"
    },
    "include_edits": {
      "default": false,
      "type": "boolean"
    },
    "include_glossary": {
      "default": true,
      "type": "boolean"
    },
    "include_parent_grounding": {
      "default": true,
      "description": "Attach parent grounding before freezing (see get_thread_context).",
      "type": "boolean"
    },
    "message_limit": {
      "maximum": 500,
      "minimum": 1,
      "type": "integer"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    },
    "token_budget": {
      "description": "Cap the message page by estimated tokens before freezing (see get_thread_context).",
      "minimum": 1,
      "type": "integer"
    },
    "transition_limit": {
      "maximum": 200,
      "minimum": 1,
      "type": "integer"
    }
  },
  "required": [
    "thread_id"
  ],
  "type": "object"
}
```

### `get_workspace_context`

Pack workspace channels, thread contexts (bounded by thread_limit), and the workspace glossary (once at the top level).

**Capability:** `workspace:read`

```json
{
  "properties": {
    "include_glossary": {
      "default": true,
      "description": "Include the workspace glossary once at the top level (grounding); omitted when empty. Set false to drop it.",
      "type": "boolean"
    },
    "message_limit": {
      "maximum": 500,
      "minimum": 1,
      "type": "integer"
    },
    "thread_limit": {
      "maximum": 50,
      "minimum": 1,
      "type": "integer"
    },
    "token_budget": {
      "description": "Cap each nested thread's message page by estimated tokens (see get_thread_context). Omit to cap by rows only.",
      "minimum": 1,
      "type": "integer"
    },
    "transition_limit": {
      "maximum": 200,
      "minimum": 1,
      "type": "integer"
    },
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id"
  ],
  "type": "object"
}
```

### `request_approval`

Human-in-the-loop gate: open a durable approval gate and return {status: input_required, gate_id} without blocking. A human resolves it later (accept/decline/cancel) over the /ui; poll get_approval_gate for the outcome. Silence is never consent. Pass thread_id to make it a claim gate — while the gate is pending, claim_next will not hand that thread to an agent.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "prompt": {
      "description": "what the human is being asked to approve",
      "type": "string"
    },
    "schema": {
      "description": "optional JSON Schema for structured detail the human may supply alongside their decision",
      "type": "object"
    },
    "thread_id": {
      "description": "optional thread to gate: while pending, claim_next skips this thread (a required-human claim gate)",
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "prompt"
  ],
  "type": "object"
}
```

### `get_approval_gate`

Poll a durable approval gate by id. Returns the gate — state is pending until a human answers, then accepted/declined/cancelled with any content they supplied — or null if no such gate exists in your workspace.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "gate_id": {
      "description": "the gate id returned by request_approval",
      "type": "string"
    }
  },
  "required": [
    "gate_id"
  ],
  "type": "object"
}
```

### `link_slack_channel`

Link a Slack channel to a Maidan thread so the projector bridges messages both ways. The workspace/channel and attribution member come from the authenticated caller and thread. Requires workspace:write + access to the thread.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "slack_channel_id": {
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "slack_channel_id"
  ],
  "type": "object"
}
```

### `list_slack_channel_links`

List the Slack channel links in your workspace. Requires workspace:read.

**Capability:** `workspace:read`

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### `unlink_slack_channel`

Remove a Slack channel link in your workspace. Returns {unlinked: bool} (false if no such link belongs to your workspace). Requires workspace:write.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "slack_channel_id": {
      "type": "string"
    }
  },
  "required": [
    "slack_channel_id"
  ],
  "type": "object"
}
```

### `link_github_issue`

Link a GitHub issue/PR to a Maidan thread so the projector bridges messages both ways. The workspace/channel and attribution member come from the authenticated caller and thread. Requires workspace:write + access to the thread.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "issue_number": {
      "type": "integer"
    },
    "repo": {
      "description": "owner/name",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "repo",
    "issue_number"
  ],
  "type": "object"
}
```

### `list_github_issue_links`

List the GitHub issue/PR links in your workspace. Requires workspace:read.

**Capability:** `workspace:read`

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### `unlink_github_issue`

Remove a GitHub issue/PR link in your workspace. Returns {unlinked: bool} (false if no such link belongs to your workspace). Requires workspace:write.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "issue_number": {
      "type": "integer"
    },
    "repo": {
      "description": "owner/name",
      "type": "string"
    }
  },
  "required": [
    "repo",
    "issue_number"
  ],
  "type": "object"
}
```

## Resources

### `workspace` — `maidan://workspaces/{id}`

Workspace metadata.

### `channel` — `maidan://channels/{id}`

Channel metadata.

### `thread` — `maidan://threads/{id}`

Full thread transcript (up to 100 messages).

### `artifact` — `maidan://artifacts/{sha256}`

Artifact metadata and byte length (body omitted).

## Prompts

### `thread_workflow`

Suggested agent steps for a thread based on its FSM state.

**Arguments:**

```json
[
  {
    "description": "Thread UUID",
    "name": "thread_id",
    "required": true
  }
]
```

