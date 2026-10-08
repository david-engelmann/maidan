# MCP reference

Auto-generated from `maidan-mcp` `tools/list`, `resources/templates/list`, and `prompts/list` catalogs. Regenerate with `cargo run -p maidan-mcp --bin gen-mcp-reference`.

## Transport

- **Protocol revisions:** `2026-07-28` (default), `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`. `initialize` echoes the revision you request if it is one of these
- **HTTP:** `POST /mcp` (JSON-RPC 2.0). `POST /mcp/worker` and `POST /mcp/reviewer` serve a fixed tool profile: `tools/list` is sorted and the same bytes for every caller (`cacheScope: "public"`), and a tool the token cannot call is refused at `tools/call`
- **HTTP notifications:** `GET /mcp/notifications` (SSE JSON-RPC notifications)
- **Streamable HTTP:** `POST /mcp/streamable` — every revision from `2025-03-26` on is stateless: one JSON-RPC response per POST, no `Mcp-Session-Id`, a notification answered `202`; optional SEP-2243 `Mcp-Method`/`Mcp-Name` routing headers. Only a `2024-11-05` client (by `initialize` or `MCP-Protocol-Version`) gets the SSE-session model (the first request opens the SSE + `Mcp-Session-Id`; follow-ups with that id are pushed to the session). Server→client messages ride `GET /mcp/streamable` or `GET /mcp/stream`
- **SSE:** `GET /mcp/stream` for workspace event stream replay/live
- **stdio:** `maidan mcp-stdio` for desktop clients (SQLite or Postgres `DATABASE_URL`; `resources/subscribe` notifications). Set `MAIDAN_MCP_TOKEN`: it scopes every tool the process serves, and without it the command refuses unless `--allow-insecure-no-auth` is passed

Bearer token required unless `AUTH_DISABLED=1`.

## JSON-RPC methods

- `initialize`, `server/discover` (revisions, capabilities and instructions with no handshake)
- `tools/list`, `tools/call`
- `resources/list`, `resources/templates/list`, `resources/read`, `resources/subscribe`, `resources/unsubscribe`
- `prompts/list`, `prompts/get`

Every result carries `resultType: "complete"`. The results of `server/discover`, `tools/list`, `prompts/list`, `resources/list`, `resources/templates/list` and `resources/read` carry a `ttlMs` and a `cacheScope` (SEP-2549); the value for each is tabled in Protocols, "MCP discovery and cache hints". `server/discover`, and an `initialize` negotiated to `2026-07-28`, omit `resources.subscribe` (on that revision the flag means `subscriptions/listen`, which is not implemented). An earlier revision's `initialize` still sets it.

**Notification:** `notifications/resources/updated` with `{ "uri": "maidan://..." }`. Mutating tools fan out to related thread/channel/workspace/artifact URIs.

A subscription belongs to the caller that made it, in the session it made it in, and an update reaches it only from its own workspace and only while it can still read the resource (private channels and DMs included; losing access ends the subscription). Subscribing to a resource you cannot read is refused like `resources/read`. Where it arrives:

- **stdio:** after each response.
- **Stateless HTTP** (`POST /mcp`, or `POST /mcp/streamable` from `2025-03-26` on): on your own `GET /mcp/notifications` or `GET /mcp/streamable` listener, on any replica: stateless subscriptions are kept in the database, so the replica holding your listener delivers them wherever you subscribed.
- **`2024-11-05` session:** on that session's stream (or `GET /mcp/streamable` with its `Mcp-Session-Id`), until it is closed or expires.

Subscriptions end with their session; a stateless caller's end once it has had no listener on any replica for the session TTL (`MAIDAN_MCP_STREAMABLE_SESSION_TTL_SECS`, default 3600). A caller may watch at most 1024 resources at once.

## Where a channel or a task comes from

An agent creates a channel with `create_channel` and a thread with `create_thread`. Both need `workspace:write`. A thread also needs access to its channel. The same writes are `POST /workspaces/{wid}/channels` and `POST /channels/{cid}/threads`, and a person can create either in the web UI. Two other tools also make threads, from a recipe or a timer: `instantiate_recipe` builds a parent and its children from a recipe, and `create_task_schedule` creates a thread each time its schedule fires.

## Tools

Every tool's `annotations` carry a `title` and the four hints of the MCP tool spec. `readOnlyHint` is true only for a tool that writes nothing, not even an audit row. `destructiveHint` is true for one that deletes, revokes, or replaces a stored value. `idempotentHint` is true when repeating the same call changes nothing further, and `openWorldHint` when the tool reaches outside Maidan (an HTTP receiver, Slack, GitHub, an embedding provider). The reason for each value is reviewed in `crates/maidan-mcp/tests/fixtures/tool-annotations.json`, and `tool_annotations_contract` fails when a tool's hints and that table disagree.

### `whoami`

**Who am I.** Return the authentication-bound identity: actor_id, member_id, optional delegation_grant_id, workspace_id, capabilities, capability_sets the caller fully holds, and whether the credential is a bearer. Call this first — writes are attributed to member_id.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `list_capability_sets`

**List capability sets.** List named capability sets (maidan.agent.worker, maidan.human.admin) and the atomic capabilities each expands to at mint time.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `parse_maidan_uri`

**Parse Maidan URI.** Parse a hierarchical maidan:// room URI (workspace UUID authority, then channels, threads, messages). The authority must be a workspace UUID, not a handle. Optional sha256 fragment is a content hash. MCP thread resource URIs and event pins are rejected.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get room.** Get the authenticated room card for a workspace: stable UUID URI plus the current handle alias. A handle rename does not change the URI.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Set workspace handle.** Set or rename a workspace handle alias. Stored ids and maidan:// URIs keep using the workspace UUID. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Attenuate token.** Derive a weaker API token from the caller's grant without token:admin (Levy/Madden attenuation). capabilities must be a non-empty subset of what the caller holds. A derived expires_at cannot outlive the parent bearer. Returns the new token secret once.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

### `rotate_token`

**Rotate token.** Replace the secret of the bearer token this call is made with. The new token keeps the same member, capabilities, expiry, quotas and derived tokens; the old secret stops working immediately. Returns the new secret once. Takes no arguments. A delegated token cannot be rotated; exchange its grant again.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### `delegate_token`

**Delegate token.** Exchange a durable delegation grant for a short-lived token acting as its subject. The token defaults to 15 minutes, cannot exceed one hour or its grant/parent bearer, and is limited to the intersection of grant and delegate capabilities.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**Create delegation grant.** Create an expiring capability-scoped grant authorizing one workspace member to delegate actions for another. Requires token:admin and a non-empty purpose.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**List delegation grants.** List delegation grants in a workspace, including expiry and revocation state. Requires token:admin.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Revoke delegation grant.** Revoke a delegation grant and every exchanged token and attenuated descendant. Requires token:admin.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Open DM conversation.** Open the 1:1 DM conversation between the authenticated member and another workspace member. The first call creates the conversation and its thread, and a repeat returns the same conversation.

**Capability:** `message:post`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List DM conversations.** List DM conversations for a member in a workspace.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Post DM message.** Post a message in a DM conversation.

**Capability:** `message:post`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

```json
{
  "anyOf": [
    {
      "required": [
        "body"
      ]
    },
    {
      "required": [
        "content"
      ]
    }
  ],
  "properties": {
    "body": {
      "description": "plain text; omit when sending typed content (body is derived from it)",
      "type": "string"
    },
    "content": {
      "description": "typed content blocks: {type: text|code|tool_use|tool_result|resource_link, ...}",
      "items": {
        "properties": {
          "type": {
            "enum": [
              "text",
              "code",
              "tool_use",
              "tool_result",
              "resource_link"
            ],
            "type": "string"
          }
        },
        "required": [
          "type"
        ],
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
    "dm_conversation_id"
  ],
  "type": "object"
}
```

### `create_channel`

**Create channel.** Create a channel in a workspace. Requires workspace:write, the same capability as POST /workspaces/{wid}/channels. A private channel adds the caller as its admin so they are not locked out.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "name": {
      "type": "string"
    },
    "private": {
      "default": false,
      "type": "boolean"
    },
    "topic": {
      "type": "string"
    },
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id",
    "name"
  ],
  "type": "object"
}
```

### `list_channels`

**List channels.** List channels in a workspace.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Add channel member.** Add (or update the role of) a member of a channel. Requires channel:admin. Private channels are gated to their members.

**Capability:** `channel:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**List channel members.** List the members of a channel. Requires channel:admin.

**Capability:** `channel:admin`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Remove channel member.** Remove a member from a channel. Requires channel:admin.

**Capability:** `channel:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

### `create_thread`

**Create thread.** Create a thread in a channel. Requires workspace:write and access to the channel, the same rule as POST /channels/{cid}/threads. title and parent_thread_id are optional. A spawn the budget refuses is refused here too.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "parent_thread_id": {
      "format": "uuid",
      "type": "string"
    },
    "title": {
      "type": "string"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `list_threads`

**List threads.** List a channel's live threads, oldest first, keyset-paginated. Default 100 (max 500); pass cursor=<last thread id of the prior page> for the next page.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List child threads.** A parent thread's child threads, each collapsed to a summary with a message count — a threaded view of 'N replies' per child without loading each child's messages.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List recently active threads.** A channel's threads ordered by last activity — most-recently-posted first. A post floats its thread to the top; a rename does not.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Mute thread.** Mute a thread for yourself — the notification router stops routing this thread's activity to you, without leaving the channel or thread. Idempotent.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Unmute thread.** Unmute a thread you previously muted. Returns unmuted=false if it was not muted.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Mute channel.** Mute a whole channel for yourself — the notification router stops routing its firehose (new-message notifications) to you, without leaving the channel. A mention still breaks through. Idempotent.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Unmute channel.** Unmute a channel you previously muted. Returns unmuted=false if it was not muted.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Set thread budget.** REPLACE a thread's whole budget envelope. Every dimension must be stated — max_tokens, max_usd_micros ($1 = 1000000), max_turns, max_wall_secs — and null means no cap on that dimension. Omitting one is an error rather than a silent removal, because a removed cap never binds and the run it should have stopped keeps going. Use update_thread_budget to change some dimensions and leave the rest alone. An unrecognized key is rejected rather than ignored. Accumulated usage is preserved. When a dimension is exceeded, report_usage stops the run.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Update thread budget.** Change only the budget dimensions you name, leaving the rest as they are. An omitted dimension is untouched; an explicit null clears that cap. Use this to raise or lower one limit without restating the others — set_thread_budget replaces the whole envelope. An unrecognized key is rejected rather than ignored.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Get thread budget.** A thread's budget envelope with accumulated usage, or null if none is set.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Report usage.** Record one retry-safe usage heartbeat for your active claim. input is uncached input. Cache writes are a 5-minute tier and a 1-hour tier. Reuse usage_report_id only for an exact retry; claim_lease_id fences stale workers. Maidan derives reporter from auth and payer from the thread. The price snapshot must calculate to usd_micros. A token budget counts fresh tokens only. A binding cap atomically stops the run.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "additionalProperties": false,
  "properties": {
    "claim_lease_id": {
      "description": "active claim fencing token",
      "format": "uuid",
      "type": "string"
    },
    "evidence": {
      "additionalProperties": false,
      "properties": {
        "batch": {
          "type": "boolean"
        },
        "cache_key": {
          "maxLength": 256,
          "type": "string"
        },
        "cache_miss_reason": {
          "maxLength": 512,
          "type": "string"
        },
        "harness": {
          "maxLength": 64,
          "type": "string"
        },
        "harness_version": {
          "maxLength": 64,
          "type": "string"
        },
        "pack_sha256": {
          "items": {
            "maxLength": 64,
            "minLength": 64,
            "type": "string"
          },
          "maxItems": 32,
          "type": "array"
        },
        "provider": {
          "maxLength": 64,
          "type": "string"
        },
        "service_tier": {
          "maxLength": 64,
          "type": "string"
        }
      },
      "type": "object"
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
        "cache_write_1h_usd_micros_per_million": {
          "minimum": 0,
          "type": "integer"
        },
        "cache_write_5m_usd_micros_per_million": {
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
        "cache_write_5m_usd_micros_per_million",
        "cache_write_1h_usd_micros_per_million"
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
          "description": "cached input; not part of a token budget",
          "minimum": 0,
          "type": "integer"
        },
        "cache_write_1h": {
          "minimum": 0,
          "type": "integer"
        },
        "cache_write_5m": {
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
        "cache_write_5m",
        "cache_write_1h"
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

### `usage_rollup`

**Roll up usage.** Spend, cache hit rate, cache write share, dollars saved against the uncached price, and cost per completed task. Scope is the workspace, or one thread, or one member. A completed task is a closed or archived thread that has not been tombstoned. Rates are parts per million of prompt tokens.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "additionalProperties": false,
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
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

### `list_dlq`

**List dead-lettered runs.** A channel's agent-work dead-letter queue — runs stopped for exceeding their budget, newest first. Triage these (retry, raise the budget, give up).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get tool transcript.** A thread's tool-call transcript: every ToolUse block correlated with its ToolResult by id. A token-lean projection that drops text/code blocks and bodies.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Assign thread.** Assign or hand off a thread/task to a member, optionally with a handoff note delivered to subscribers on the assignment event.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

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

**Claim thread.** Atomically claim an unassigned thread for a member. Returns {thread, claimed}; claimed=false if it was already assigned.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Unassign thread.** Clear a thread's assignee.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

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

**Transition thread.** Advance a thread's FSM state (start_review, close, or archive). The MCP twin of REST POST /threads/:id. Separation of duties, the required-reviewers close-gate, and unresolved refutes all apply identically — there is no MCP bypass. On a thread with a review requirement, start_review is refused until a result is posted (set_thread_result). Returns the updated thread.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

```json
{
  "properties": {
    "action": {
      "description": "start_review (open to in_review), close (in_review to closed) or archive (closed to archived). To send work back, use submit_review with request_changes",
      "enum": [
        "start_review",
        "close",
        "archive"
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
    "action"
  ],
  "type": "object"
}
```

### `list_assigned_threads`

**List assigned threads.** List the threads currently assigned to a member (their work queue), oldest first.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Set wait.** Set (upsert) a wait timer on a thread: it is waiting until wait_until, and on timeout the sweeper escalates via on_timeout but decides nothing (notify reaches the owner; park also marks the thread unclaimable). Default policy is notify. Cancel it when the awaited thing happens. Requires thread:transition. This returns at once and does not block; to block until something happens, use a wait_for_* tool such as wait_for_result or wait_for_ready.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

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

**Cancel wait.** Cancel a thread's wait — the awaited thing happened (G2). {cancelled} is false when no wait was set. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Get wait.** The thread's wait timer (deadline, on_timeout policy, reason, fired_at), or null if none is set (G2).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Set priority.** Set (upsert) a thread's dispatch priority (G3 fair dispatch). Higher = more urgent (default 0). claim_next orders by an effective rank = this priority aged up the longer the thread waits, so priority jumps the queue without starving long-waiting tasks. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Get priority.** The thread's dispatch-priority record, or null (which means the default priority 0) (G3).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Mark unclaimable.** Park a thread from dispatch (G3): claim_next skips it and an explicit claim is refused, until cleared. An explicit park (needs triage, waiting on external, broken) — distinct from blocked-by-deps / blocked-by-gate / skill-miss. Reason must be non-empty. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Mark claimable.** Un-park a thread (G3) — it becomes claimable again. {cleared} is false when it was not parked. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**List unclaimable.** The parked (unclaimable) threads in a channel (G3), newest first — for triage.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Set thread block.** Set (upsert) an explicit dispatch block on a thread (G14): claim_next skips it and an explicit claim is refused, until cleared. reason is the closed enum dag|gate|human|child|quota|unclaimable — not a free string. Distinct from DAG-children-must-be-terminal. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Get thread block.** The thread's explicit dispatch block, or null when unblocked (G14). Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Clear thread block.** Clear an explicit dispatch block (G14). Emits BlockedResolved so waiters can observe the unblock. {cleared} is false when it was not blocked. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**List blocked threads.** The explicitly blocked threads in a channel (G14), newest first — for triage. Distinct from queue-depth blocked (unfinished DAG deps).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `declare_status`

**Declare thread status.** Declare the agent's self-reported status on a thread: working, needs_input, needs_review, blocked, or done, with a one-sentence note. By the claim holder or owner; stalled is refused (system-computed only). Supersedes any prior declaration. Appends StatusDeclared. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

```json
{
  "properties": {
    "note": {
      "description": "One-sentence description of what the agent is doing",
      "type": "string"
    },
    "status": {
      "enum": [
        "working",
        "needs_input",
        "needs_review",
        "blocked",
        "done"
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
    "status",
    "note"
  ],
  "type": "object"
}
```

### `get_thread_status`

**Get thread status.** The thread's active agent status declaration, or null when cleared. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `get_thread_version`

**Get thread version.** How many writes the thread's content has seen: its messages, result, title and description, and linked artifacts. The database moves it on every such write, so a decision can name the version it was shown. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `list_thread_artifacts`

**List thread artifacts.** The artifacts linked to the thread as evidence, in the order they were linked, each with who linked it. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `link_thread_artifact`

**Link thread artifact.** Link an artifact your workspace holds to a thread as evidence, by its sha256. Upload it first; a hash your workspace does not hold is not found. Linking twice keeps the first link. Moves the thread's version. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "sha256": {
      "description": "the artifact's sha256, hex",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "sha256"
  ],
  "type": "object"
}
```

### `unlink_thread_artifact`

**Unlink thread artifact.** Unlink an artifact from a thread. Returns whether it was linked. Moves the thread's version when it was. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "sha256": {
      "description": "the artifact's sha256, hex",
      "type": "string"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "thread_id",
    "sha256"
  ],
  "type": "object"
}
```

### `set_wip_limit`

**Set WIP limit.** Set or clear this workspace's WIP limit (G11): the max concurrent live claims any one member may hold. limit >= 0 caps it (0 freezes claiming); omit or null clears it (unlimited). Applies to your own workspace. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

### `set_delegation_policy`

**Set delegation policy.** Set the longest a delegation grant may live in this workspace, in days (1 to 3650); omit or null to restore the default of 90. A grant is the standing authority to keep minting delegated tokens, so this bounds real exposure. Applies to grants issued afterwards. Requires token:admin.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "additionalProperties": false,
  "properties": {
    "max_grant_days": {
      "anyOf": [
        {
          "maximum": 3650,
          "minimum": 1,
          "type": "integer"
        },
        {
          "type": "null"
        }
      ],
      "description": "grant lifetime ceiling in days; null/omit = the default, 90"
    }
  },
  "type": "object"
}
```

### `get_delegation_policy`

**Get delegation policy.** This workspace's delegation policy: max_grant_days, the longest a delegation grant may live, and is_default when the workspace has set none.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `set_retention_policy`

**Set retention policy.** Replace this workspace's own retention for its messages, events and finished deliveries, in days (1 to 3650 each). A workspace may keep rows for less time than the instance does, never longer: a value above the instance's is refused. Omit or null a kind to keep it as long as the instance does; no arguments clears the policy. Old messages are erased with their embeddings and content keys. A workspace under legal hold loses nothing whatever its policy says. Requires token:admin.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "additionalProperties": false,
  "properties": {
    "deliveries_days": {
      "anyOf": [
        {
          "maximum": 3650,
          "minimum": 1,
          "type": "integer"
        },
        {
          "type": "null"
        }
      ],
      "description": "days to keep finished deliveries; null/omit = as long as the instance"
    },
    "events_days": {
      "anyOf": [
        {
          "maximum": 3650,
          "minimum": 1,
          "type": "integer"
        },
        {
          "type": "null"
        }
      ],
      "description": "days to keep event-log rows; null/omit = as long as the instance"
    },
    "messages_days": {
      "anyOf": [
        {
          "maximum": 3650,
          "minimum": 1,
          "type": "integer"
        },
        {
          "type": "null"
        }
      ],
      "description": "days to keep messages; null/omit = as long as the instance (forever)"
    }
  },
  "type": "object"
}
```

### `get_retention_policy`

**Get retention policy.** This workspace's retention: what it set (workspace), what the instance keeps (instance), and what is pruned in effect, the shorter of the two per kind (effective). Days per kind; null means not pruned.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `get_wip_limit`

**Get WIP limit.** This workspace's WIP limit (max concurrent live claims per member), or null when unset (unlimited).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `set_spawn_budget`

**Set spawn budget.** Set this workspace's spawn budget (G6): how far an agent family may fan out. max_children caps the direct child threads per parent, max_depth the thread nesting, max_tools the tool calls recorded on one thread. A full replace — an omitted or null axis is unlimited, so calling with no arguments clears the budget; 0 freezes an axis. Keep the caps small: coordination cost grows quadratically in the number of agents. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Get spawn budget.** This workspace's spawn budget as {max_children, max_depth, max_tools}; a null axis is unlimited. Read it before spawning helpers to see how much fan-out is left.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `get_member_wip`

**Get member WIP.** A member's current live-claim count against the workspace WIP limit ({live_claims, limit}) — for backpressure decisions before claiming more work.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Claim next thread.** Atomically claim the oldest claimable thread in a channel for a member (claimable = unassigned or its lease expired). Every claim is leased. Returns the claimed thread with a content-addressed pin {uri, content_hash}, or null when there is no claimable work. This tool takes work; it does not create it. Create a channel with create_channel and a task with create_thread (both need workspace:write; the REST twins are POST /workspaces/{wid}/channels and POST /channels/{cid}/threads).

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

```json
{
  "properties": {
    "channel_id": {
      "format": "uuid",
      "type": "string"
    },
    "lease_secs": {
      "description": "lease in seconds (1 s to 7 days); renew before it lapses, or the thread is reaped and returns to the queue with a ClaimExpired. Omitted, the server's default lease applies (MAIDAN_CLAIM_DEFAULT_LEASE_SECS, 600 s)",
      "maximum": 604800,
      "minimum": 1,
      "type": "integer"
    }
  },
  "required": [
    "channel_id"
  ],
  "type": "object"
}
```

### `claim_next_workspace_thread`

**Claim next workspace thread.** Atomically claim the oldest claimable thread across every channel of the workspace you may read (claim_next_thread for the whole workspace), so an agent serving the whole workspace makes one call instead of one per channel. The same filters (open, dependencies finished, skills held, no pending approval gate, not blocked or parked unclaimable, you not frozen), order, lease and fencing token. A private channel's threads go only to its members and a DM's only to its participants. Returns the claimed thread with a content-addressed pin {uri, content_hash}, or null when there is no claimable work.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

```json
{
  "properties": {
    "lease_secs": {
      "description": "lease in seconds (1 s to 7 days); renew before it lapses, or the thread is reaped and returns to the queue with a ClaimExpired. Omitted, the server's default lease applies (MAIDAN_CLAIM_DEFAULT_LEASE_SECS, 600 s)",
      "maximum": 604800,
      "minimum": 1,
      "type": "integer"
    },
    "workspace_id": {
      "description": "your own workspace (whoami's workspace_id)",
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

### `renew_claim`

**Renew claim.** Extend a claimed thread's lease (heartbeat). Only the current assignee holding the matching fencing token may renew; a stale holder whose claim was reclaimed is rejected.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

```json
{
  "properties": {
    "claim_lease_id": {
      "description": "the fencing token from the claim response's thread.claim_lease_id",
      "format": "uuid",
      "type": "string"
    },
    "lease_secs": {
      "description": "new lease deadline in seconds from now (1 s to 7 days)",
      "maximum": 604800,
      "minimum": 1,
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

**Acknowledge claim.** Acknowledge a claimed thread and start its working clock (work_started_at): the current holder signals it has begun work, distinct from just holding the claim. Acknowledge as soon as you start: a leased claim left unacknowledged past the server's window (MAIDAN_CLAIM_ACK_TIMEOUT_SECS, 120 s) is reported to its owner with a claim_unacknowledged event. Only the assignee holding the matching fencing token may acknowledge; idempotent (the first start time is kept).

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Release claim.** Release a claim (graceful handoff): the current holder returns the thread to the queue immediately by presenting its fencing token, instead of letting the lease lapse — e.g. an agent shutting down cleanly. Only the assignee holding the matching token may release. Clears the assignment and working clock and emits thread_assignment_changed.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Add thread dependency.** Add a task-dependency edge: the thread depends on depends_on_thread_id and stays blocked (won't be handed out by claim_next) until that dependency reaches a terminal state. Both threads must be in the same workspace.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List thread dependencies.** List a task's dependencies plus whether it is ready to run (true when every dependency is terminal).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get queue depth.** A channel's task-queue depth: counts of its open task threads as {open, ready, assigned, blocked, unclaimable}, for deciding whether to scale workers. ready is what claim_next_thread could take now. Counts only threads you may read: on the DM channel, your own DMs. For the whole workspace, use get_workspace_queue_depth.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get channel occupancy.** A channel's occupancy as {open, queued, claimed, working, blocked}: the two-clocks refinement of get_queue_depth. It splits held work into claimed (an agent grabbed the task but hasn't acknowledged it via acknowledge_claim) and working (acknowledged and underway) — surfacing a claimed-but-idle agent. queued/blocked mirror get_queue_depth's ready/blocked.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `get_workspace_queue_depth`

**Get workspace queue depth.** get_queue_depth across every channel of the workspace you may read, as {open, ready, assigned, blocked, unclaimable}: the sum of those channels' depths, for sizing a pool of workers that claim with claim_next_workspace_thread. A private channel's threads count only for its members and a DM's only for its participants.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "workspace_id": {
      "description": "your own workspace (whoami's workspace_id)",
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

### `get_workspace_occupancy`

**Get workspace occupancy.** get_channel_occupancy across every channel of the workspace you may read, as {open, queued, claimed, working, blocked}: the sum of those channels' occupancy. A private channel's threads count only for its members and a DM's only for its participants.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "workspace_id": {
      "description": "your own workspace (whoami's workspace_id)",
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

### `set_thread_lineage`

**Set thread lineage.** Home a producer's run_id on a thread as parent_run_id. Accepts the producer's string as-is (does not mint a parallel id). Empty / whitespace / over-long is rejected. Use when attributing nested work to a producer run; set_thread_result also auto-homes when the payload carries run_id.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Get thread lineage.** Read a thread's run lineage (parent_run_id + set_at), or null if none has been set.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List run threads.** List threads in the caller's workspace that share a producer parent_run_id, oldest first. Nested children given the same value are included. Private-channel rows the caller cannot access are omitted. F7 mute is not consulted.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get run occupancy.** Nested occupancy for a producer run as {parent_run_id, open, queued, claimed, working, blocked}: the two-clocks partition of every open workspace thread that shares parent_run_id. F7 mute is orthogonal (a muted nested thread still counts). Unknown / unused run returns zeros.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Set thread result.** Attach a task's structured result (arbitrary JSON). Upserts one result per thread and notifies waiters via a thread_result_set event. Use when finishing a task so a requester or parent can read the output.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: true`

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

**Get thread result.** Read a task's structured result, or null if none has been produced yet.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List thread results.** List thread results in the caller's workspace, newest first. Optional result_kind is an exact-match facet on the namespaced string (e.g. example.review.result/1), not a closed enum. Private-channel rows the caller cannot access are omitted.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List result deliveries.** List per-target delivery status for a thread's structured result (disposition, external reference, last error). Empty means the result was not routed anywhere, which is valid. workspace:read + thread access.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Replay result delivery.** Re-enqueue one result delivery onto the egress outbox. Re-checks the workspace allowlist (an unblessed target stays skipped). Does not bump armed_revision. workspace:write + thread access.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: true`

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

**Set thread owner.** Set (or clear, by omitting owner_id) a thread's durable owner — the accountable party, distinct from the assignee/claimer. Once an owner is set, the claimer can no longer land (close/archive) its own work; the owner or another member must (separation of duties).

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Rename thread.** Rename a thread — give a titled thread a new name (e.g. name a post-derived child thread). The title must not be blank. A rename is metadata, not activity, so it does not float the thread in the recent-activity order.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Set thread steer.** Set (upsert) a thread's persisted steer — durable steering guidance that survives claims and handoffs, so a resuming or newly-assigned agent reads the current steer. Latest wins.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Get thread steer.** Read a thread's current steer, or null if none is set. A resuming or newly-assigned agent reads this to follow the current steering guidance.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Wait for result.** Block until a task's result is produced (a thread_result_set event for thread_id), returning the result payload, or null on timeout. The coordination wait for spawn/wait/aggregate. Pass since_log_id (your high-water log_id) to also catch a result set in the gap before this call subscribes; omit it for pure-live (read get_thread_result first for an already-produced result). To put a deadline on a thread instead, use set_wait.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get dependency results.** Gather the structured results of a parent task's dependencies as a list of {thread_id, result} objects (result null if not produced yet), skipping dependencies you can't access. The spawn/wait/aggregate read for a parent task.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Add member skill.** Declare a skill (free-form tag) for a member. Skill routing gates claim_next: a task is claimable by a member only if it holds all the task's required skills.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List member skills.** List a member's declared skills.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Add thread required skill.** Add a required skill to a task. Only a member holding every required skill can claim the task via claim_next_thread.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List thread required skills.** List a task's required skills.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Create task schedule.** Create a task schedule: when due, the sweeper creates a thread titled `title` in `channel_id` (or, when recipe_id is set, instantiates that recipe — parent + DAG children — instead). interval_secs omitted = one-shot; a positive value = recurring. first_run_at omitted = fire on the next tick.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**List task schedules.** List the caller's workspace task schedules (filtered to channels the caller can access).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `create_recipe`

**Create recipe.** Create a recipe: a reusable thread-type blueprint. spec = {params, definition_of_done, retry, children}, where each child is {key, title, required_skills, depends_on (sibling keys)}. Instantiating it (instantiate_recipe) builds a parent thread + a child per child + wires the DAG + attaches skills. NOT a recipe VM — a blueprint the room instantiates.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**List recipes.** List the caller's workspace recipes (filtered to channels the caller can access).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `instantiate_recipe`

**Instantiate recipe.** Instantiate a recipe into a parent thread + its DAG children (copy-on-fire: the recipe bytes are frozen into the run). params are validated against the recipe's declared params (required ones must be present). Returns the RecipeRun.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**List secrets.** List the caller's workspace secrets (metadata only — id, name, timestamps; NEVER the value). Use resolve_secret to fetch a value at exec.

**Capability:** `secret:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `resolve_secret`

**Resolve secret.** Resolve a named secret to its value (the 'fetch at exec' path). The value is decrypted server-side and returned only in this response — it never enters the event log. Returns null-name error if the secret is unknown.

**Capability:** `secret:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `list_secret_egress_hosts`

**List secret egress hosts.** List the hosts trusted with this workspace's secret values: on a webhook, automation HTTP or A2A push delivery to one of them, secret://<name> refs in the payload are replaced with the workspace's values; any other host gets the literal ref. Empty (the default) means no host gets a value. Requires secret:admin.

**Capability:** `secret:admin`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `allow_secret_egress_host`

**Allow secret egress host.** Trust a host with this workspace's secret values (idempotent). A listed host receives the value of every secret a payload bound for it names, so this needs secret:read as well as secret:admin. The host is a lowercase hostname or IPv4 address with no scheme, port, path or wildcard, and must be inside the instance ceiling when the operator set one. Audited.

**Capability:** `secret:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "host": {
      "description": "e.g. hooks.example.com",
      "type": "string"
    }
  },
  "required": [
    "host"
  ],
  "type": "object"
}
```

### `revoke_secret_egress_host`

**Revoke secret egress host.** Stop trusting a host with this workspace's secret values; the next delivery to it carries the literal secret:// refs. Not found when the host was not listed. Requires secret:admin. Audited.

**Capability:** `secret:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "host": {
      "type": "string"
    }
  },
  "required": [
    "host"
  ],
  "type": "object"
}
```

### `freeze_member`

**Freeze member.** Freeze a member (the kill-switch): drops their active leases (releases their claimed threads) and makes claim_next refuse them. Returns the freeze record + the count released, and emits member_frozen to the workspace (the reason included). The member stays frozen until unfreeze_member. Requires token:admin. NOT a thread/workspace pause.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

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

**Unfreeze member.** Lift a member's freeze so they can claim work again. Requires token:admin. Returns {unfrozen} (false if they were not frozen); an unfreeze that lifts a freeze emits member_unfrozen.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**List frozen members.** List the frozen members in the caller's workspace (member_id, frozen_at, frozen_by, reason). Requires token:admin.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `create_share_ticket`

**Create share ticket.** Issue a read-only cross-organization ticket for one channel and an explicit artifact allowlist. Ownership is bound to the authenticated member. Lifetime is capped at 48 hours; the secret is returned once and only its hash is stored. Requires token:admin.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**List share tickets.** List share tickets and their explicit artifact scopes in the caller's workspace. Secrets are never returned after creation. Requires token:admin.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `revoke_share_ticket`

**Revoke share ticket.** Immediately revoke a share ticket in the caller's workspace. Requires token:admin.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Export workspace.** Export a workspace as a signed maidan.workspace.export/1 envelope. Tokens die on export: API tokens and secrets are omitted. A blank instance can verify the file without calling this host. Requires token:admin and MAIDAN_EXPORT_SIGNING_KEY.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Verify workspace export.** Verify a signed workspace export without importing it. Fail-closed on tamper, a bad signature, stuffed secret fields, or a public key outside MAIDAN_EXPORT_VERIFY_KEYS when that pin is set. An empty pin checks integrity against the embedded key only. Requires token:admin.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Import workspace.** Verify then import a signed workspace export. mode new remaps ids into a fresh workspace; restore keeps original ids and fails if that workspace exists unless force is true. Tokens die on export: mint new tokens after import. Requires token:admin.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

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

**Get log snapshot.** Hashed event-log snapshot for this workspace (getRepo-shaped, not MST/CAR). Header plus graph_hash is workspace:read. Pass include_graph true for the domain graph; that requires token:admin. Complements hash-chain verify of the retained suffix: this covers a pruned prefix so a peer can resume without trusting the host for history it never saw.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Catch up on events.** Since-LSN catch-up page after a snapshot (or a prior page). Events have id greater than after_lsn, hash-chain checked from the predecessor. A pruned-gap cursor fails closed and names the snapshot path to refetch; a broken chain fails closed. Requires workspace:read.

**Capability:** `token:admin`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Verify event chain.** Verify the retained event-log hash chain for a workspace. Returns the chain report when intact; fails closed on a splice or rewrite. Twin of GET /workspaces/{id}/events/verify. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List tombstones.** Tombstone and deletion explorer for this workspace. Soft-deleted messages (body already cleared) plus, when include_purged is true, hard-purged reconstructions from MessageTombstoned events. Private-channel and DM rows the caller cannot access are omitted. Twin of GET /workspaces/{id}/tombstones. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List message backlinks.** Incoming pointers at a message: RelationKind reverse edges plus pins, reactions, and votes. Mentions are outgoing and omitted. Works on a retained tombstone; fails not-found after hard purge. Twin of GET /messages/{id}/backlinks. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get kind census.** EventKind counts for a workspace, optionally narrowed to a channel or thread. Inaccessible private channels are excluded from the totals. Twin of GET /workspaces/{id}/kind-census. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Create memory block.** Create a labeled memory block — a Letta-shaped shared object {label, description, limit, read_only, value} in the workspace that a thread can attach to (a room object). It is how a parent watches a child's result block without a nested runtime: not a transcript, not RAG. Concurrent-safe on the label (re-creating a label returns the existing block). The caller owns it.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get memory block.** Get a memory block by label within the caller's workspace, or null if none. This is the watch-a-child's-result-block read (poll it).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List memory blocks.** List the memory blocks in the caller's workspace (id, label, description, limit, read_only, value, owner).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `set_memory_block_value`

**Set memory block value.** Full-rewrite a memory block's value by label (last-writer-wins). A read-only block or a value over the block's char limit is rejected. Use this to publish a result other threads watch.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

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

**Attach memory block.** Attach a memory block (by label) to a thread so the thread carries it as a room object — how a parent shares a block with a child. Idempotent. Returns {attached}.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Detach memory block.** Detach a memory block (by id) from a thread. Idempotent. Returns {detached} (false if it was not attached).

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**List thread memory blocks.** List the memory blocks attached to a thread (its room objects), by label.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Wait for memory block.** Block until a memory block (by label) is rewritten in the caller's workspace, or the timeout lapses. Returns the block with its fresh value, or null on timeout. This is how a parent watches a child's result block without a nested runtime. Live: only sees updates after subscribing, so read the current value with get_memory_block first.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Set review requirement.** Set (upsert) a thread's required-reviewers gate (G5): required_count distinct qualifying approvals before it can close. An approval qualifies when the reviewer is neither the owner nor the assignee (separation of duties) and, when a named reviewer set exists, is in it. A refutes edge also blocks close. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Add reviewer.** Name a reviewer for a thread (G5) — the eligible set. Empty set = open review (any qualifying member). Idempotent. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Submit review.** Submit a review decision as the caller (G5): approve or request_changes. The reviewer is you; an owner/assignee may submit but it will not count toward the requirement (separation of duties). request_changes on an in_review thread, from its owner or a reviewer whose approval would count, sends it back to open for rework: it is claimable again, earlier approvals are dismissed, and your note appears in its context as change_requests. request_changes needs a note saying what to change; approve may carry one. Every verdict appends a review_submitted event (the note stays in the review history), and a request_changes notifies the thread's last worker. Re-submitting changes your decision. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

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
      "description": "what to change, required on request_changes; optional on approve",
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

**Get review status.** Read a thread's review standing: required_count, approvals (distinct qualifying), and approvals_met. This is the approval side of the close-gate; a refutes edge is checked separately when closing.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List reviews.** List a thread's review decisions (reviewer, decision, note).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `list_review_history`

**List review history.** List every verdict submitted on a thread, oldest first (reviewer, decision, note, delegate, recorded_at). list_reviews shows each reviewer's current decision; a re-submission replaces it there but its earlier verdicts stay here.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Set land gate.** Record a land-gate pointer on a thread: status pass or fail, optional artifact_sha, optional land green/amber/red. The room holds the pointer; an external verifier records pass/fail. A qualifying green pass (land-gate-skilled member who is not the implementer) is required to close once the gate is armed. Amber is flags-then-still-engages and is not a land. Requires thread:transition. The caller must have declared the land_gate skill.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

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

**Get land gate.** Read a thread's land-gate standing: required, pointer, land (green/amber/red), landable. No pointer is vacuous green. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `list_land_gate_history`

**List land gate history.** List every land-gate verdict recorded on a thread, oldest first (status, land, artifact_sha, recorder, delegate, recorded_at). get_land_gate shows the latest pointer; a later pointer or clearing the gate leaves earlier verdicts here. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Require land gate.** Arm the land-gate close-gate on a thread without a pointer yet so closed refuses until a qualifying green pass arrives. Idempotent. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Clear land gate.** Clear a thread's land-gate pointer and requirement. Requires thread:transition.

**Capability:** `channel:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Set glossary term.** Define (or redefine) a term in the workspace's shared glossary — the canonical term -> definition so agents use words the same way (the anti-drift pin; the target of a `defines` reference). Upserts on the term.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Get glossary term.** Look up one term's canonical definition in the workspace glossary. Returns null when the term is undefined.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List glossary terms.** List all defined terms in the workspace's shared glossary, ordered by term.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `wait_for_ready`

**Wait for ready.** Block until a task becomes ready (its last blocking dependency reaches a terminal state, emitting thread_ready), or the timeout lapses. Returns the ThreadReady event, or null on timeout. Scoped to channel_id when given, else any accessible thread in the workspace. Pass since_log_id (your high-water log_id) to also catch readiness signalled in the gap before this call subscribes; omit it for pure-live (pick up already-ready work with claim_next_thread first). To put a deadline on a thread instead, use set_wait.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Wait for claim expired.** Block until a claim's lease lapses and its thread is reclaimed (by the claim reaper within seconds, or by the next claim_next_thread; either emits claim_expired), or the timeout lapses. A supervisor's 'an agent died' signal: returns the ClaimExpired event (its member_id is the dead holder), or null on timeout. Scoped to channel_id when given, else any accessible thread in the workspace. Pass since_log_id (your high-water log_id) to also catch an expiry reclaimed in the gap before this call subscribes; omit it for pure-live. A lease on a thread in review is not reaped and emits nothing. To put a deadline on a thread instead, use set_wait.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `wait_for_claim_failed`

**Wait for claim failed.** Block until a claimed run is stopped for going over its budget (report_usage past a tokens/usd/turns/wall limit releases the claim, dead-letters the run and emits claim_failed), or the timeout lapses. Returns the ClaimFailed event (member_id is the stopped holder, reason the budget axis), or null on timeout. Scoped to channel_id when given, else any accessible thread in the workspace. Pass since_log_id (your high-water log_id) to also catch a stop in the gap before this call subscribes; omit it for pure-live. To put a deadline on a thread instead, use set_wait.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `wait_for_blocked_resolved`

**Wait for blocked resolved.** Block until an explicit dispatch block is cleared (clear_thread_block emits blocked_resolved), or the timeout lapses. Returns the BlockedResolved event (the reason that cleared, resolved_by), or null on timeout. Scoped to thread_id and/or channel_id when given, else any accessible thread in the workspace. Clearing a thread that was not blocked emits nothing. Pass since_log_id (your high-water log_id) to also catch a clear in the gap before this call subscribes; omit it for pure-live. To put a deadline on a thread instead, use set_wait.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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
      "description": "optional: wait for this thread's block to clear",
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

### `wait_for_landed`

**Wait for landed.** Block until a thread's linked GitHub PR lands (is merged, emitting thread_landed), or the timeout lapses. Returns the ThreadLanded event (repo, pr_number, merged_by, merge_commit_sha, title), or null on timeout. Scoped to thread_id and/or channel_id when given, else any accessible land in the workspace. The room records the landing but does NOT transition the thread's FSM. Pass since_log_id (your high-water log_id) to also catch a land emitted in the gap before this call subscribes; omit it for pure-live. Live-only; the GET /mcp/stream SSE transport (kinds=thread_landed) is the resumable alternative. To put a deadline on a thread instead, use set_wait.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List mentions.** List recent @mentions of a member (most recent first).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get inbox.** A member's mention inbox: recent mentions plus the read-cursor, so an agent can find what it hasn't seen. Mentions only; for everything waiting on the member (assigned tasks, requested reviews, open approval gates and unread mentions), use get_waiting_inbox.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Mark inbox read.** Advance a member's inbox read-cursor through an instant (RFC 3339); returns the updated inbox.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Wait for mention.** Block until the member is next @mentioned, or the timeout lapses. Returns the mention event, or null on timeout. Pass since_log_id (your high-water log_id from the last drain) to also catch a mention recorded in the gap before this call subscribes; omit it for pure-live behaviour (drain existing ones with get_inbox first).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List notifications.** List a member's per-recipient notifications, newest first. Set unread_only to see just the unread ones. The durable inbox the notification router fills; drain it here, then wait_for_notification for new ones.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get waiting inbox.** The waiting-on-you inbox, everything needing a member's attention: their assigned non-terminal threads, the reviews requested from them (review_request: a thread in review naming them as a reviewer, without their approval yet), the reviews that name no reviewer and fall to them (unassigned_review: a thread in review with no named reviewer that they own, or that nobody owns when they are a workspace admin), the workspace's pending approval gates, and their unread mentions. Oldest-waiting first, each aged against sla_secs (default 86400 = 24h) with an overdue flag. One member's queue, not @everyone. For mentions alone, with a read-cursor you advance with mark_inbox_read, use get_inbox.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List notifications grouped.** A member's notifications collapsed into per-thread groups, newest-activity first — a busy thread shows as one group (with its count, unread_count, and latest notification) instead of flooding the flat list. limit bounds how many notifications are scanned.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List buried decisions.** A member's buried decisions — task results (decisions) produced by someone else in a channel or thread the member follows, since a given instant (default 7 days ago), newest first. The decisions the digest surfaces, queryable directly.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get manager digest.** Compose this member's unread followed-member lifecycle notifications since an instant (default 7 days ago) into per-channel result, gate, and stuck counts. This is a notification view, not analytics.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get unread count.** A member's unread-notification badge count.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Mark notification read.** Mark one of a member's notifications read (recipient-scoped; marked=false if the id isn't this member's).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Snooze notification.** Snooze one of a member's notifications until an RFC 3339 instant — it drops out of the inbox and unread badge until then, and resurfaces once the snooze lapses. Recipient-scoped (snoozed=false if the id isn't this member's).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Wait for notification.** Block until the member gets a new notification-worthy event (today: mentions), or the timeout lapses. The general form of wait_for_mention. Returns the triggering event, or null on timeout. Pass since_log_id (your high-water log_id from the last drain) to also catch an event from the gap before this call subscribes; omit it for pure-live behaviour (drain with list_notifications first).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Set notification pref.** Set a member's mute preference for an event kind (kind is snake_case, e.g. mention_recorded). When muted, the router stops writing notifications of that kind for this member.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List notification prefs.** List a member's notification preferences (per-kind mute flags).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Set delivery mode.** Set a member's email delivery mode: immediate (a per-notification email) or digest (a periodic rollup instead). The two are mutually exclusive.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get delivery mode.** Get a member's email delivery mode (immediate when never set).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Set member email.** Set a member's delivery email address (where their email notifications go). A light @ check; full validation happens at send.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Get member email.** Get a member's delivery email address (null when unset).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Delete member email.** Clear a member's delivery email address (opt out of email). Returns {deleted}.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Follow channel.** Follow a channel so the member is notified of new messages there even without a mention (honors mutes). Requires access to the channel.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Unfollow channel.** Stop following a channel (removed=false if not following).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List channel follows.** List the channels a member follows.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Follow thread.** Follow a thread so the member is notified of new messages in it even without a mention (honors mutes). Requires access to the thread.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Unfollow thread.** Stop following a thread (removed=false if not following).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List thread follows.** List the threads a member follows.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Follow member.** Follow another same-workspace member's work occupancy. Self-follow is rejected.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get member occupancy.** Get a member's live occupancy: ephemeral presence plus assigned non-terminal threads visible to the caller.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Unfollow member.** Stop following another member's work occupancy (removed=false if not following).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List member follows.** List the member-occupancy subscriptions owned by a member.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List messages.** List messages in a thread.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Post message.** Post a message to a thread as the authenticated member.

**Capability:** `message:post`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: true`

```json
{
  "anyOf": [
    {
      "required": [
        "body"
      ]
    },
    {
      "required": [
        "content"
      ]
    }
  ],
  "properties": {
    "body": {
      "description": "plain text; omit when sending typed content (body is derived from it)",
      "type": "string"
    },
    "content": {
      "description": "typed content blocks: {type: text|code|tool_use|tool_result|resource_link, ...}",
      "items": {
        "properties": {
          "type": {
            "enum": [
              "text",
              "code",
              "tool_use",
              "tool_result",
              "resource_link"
            ],
            "type": "string"
          }
        },
        "required": [
          "type"
        ],
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
    "thread_id"
  ],
  "type": "object"
}
```

### `seed_from_message`

**Seed from message.** Seed a new titled work thread from a source message (the write side of 're-ask'), linked by a seeded_from reference edge. inclusion: 'pointer' (default, edge only) or 'quote' (a first message quoting the source). The source is untouched; N seeds per source. Lineage is queryable via list_references (dst=the source, relation=seeded_from).

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**Edit message.** Edit your own message (message:post). Only the author can edit a message; another member's message can be tombstoned, not rewritten.

**Capability:** `message:post`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

```json
{
  "anyOf": [
    {
      "required": [
        "body"
      ]
    },
    {
      "required": [
        "content"
      ]
    }
  ],
  "properties": {
    "body": {
      "description": "plain text; omit when sending typed content (body is derived from it)",
      "type": "string"
    },
    "content": {
      "description": "typed content blocks: {type: text|code|tool_use|tool_result|resource_link, ...}",
      "items": {
        "properties": {
          "type": {
            "enum": [
              "text",
              "code",
              "tool_use",
              "tool_result",
              "resource_link"
            ],
            "type": "string"
          }
        },
        "required": [
          "type"
        ],
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
    "message_id"
  ],
  "type": "object"
}
```

### `record_mention`

**Record mention.** Mark a member as mentioned in a message.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**Cast vote.** Cast a vote on a message. kind is approve, request_changes, or ack. Any other kind is rejected. An emoji is a reaction, not a vote kind. You hold at most one verdict per message: approve replaces your request_changes and the other way round, and ack stands beside either. Optional confidence (0..1) for weighted consensus; re-casting the same kind updates your confidence. retract_vote takes a vote back.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

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
      "description": "approve, request_changes, or ack. Any other kind is rejected",
      "enum": [
        "approve",
        "request_changes",
        "ack"
      ],
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

### `retract_vote`

**Retract vote.** Take back your own vote of one kind on a message. Removing a vote you do not hold changes nothing. Returns whether a vote was removed.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "kind": {
      "description": "the kind of your vote to take back",
      "enum": [
        "approve",
        "request_changes",
        "ack"
      ],
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

**Add reaction.** Add an emoji reaction to a message.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**Remove reaction.** Remove an emoji reaction from a message.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List reactions.** List emoji reactions on a message.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Pin message.** Pin a message to a thread.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**Unpin message.** Unpin a message from a thread.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List pins.** List pinned messages in a thread.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Add reference.** Add a typed reference between two threads or messages.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List references.** List references FROM a source (forward) or TO a target (reverse — 'what references this'), optionally filtered by relation. Provide exactly one of the src_kind+src_id or dst_kind+dst_id pair.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Upload artifact.** Store bytes in the artifact substrate and register metadata.

**Capability:** `artifact:upload`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: false`, `openWorldHint: false`

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

**Begin multipart artifact upload.** Start an S3 multipart upload for a large artifact (requires S3 backend).

**Capability:** `artifact:upload`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

```json
{
  "properties": {},
  "type": "object"
}
```

### `upload_artifact_multipart_part`

**Upload multipart artifact part.** Upload one part of an in-progress multipart artifact.

**Capability:** `artifact:upload`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Complete multipart artifact upload.** Finish multipart upload, content-address bytes, and register artifact metadata.

**Capability:** `artifact:upload`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Abort multipart artifact upload.** Abort a failed multipart upload.

**Capability:** `artifact:upload`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Get artifact metadata.** Fetch artifact metadata by sha256 hex digest.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Search messages.** Full-text, semantic, or hybrid search over a workspace's messages. Returns ranked hits with highlighted snippets, each naming its channel (channel_name), thread (thread_title) and author (author_handle) beside their ids.

**Capability:** `search:query`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: true`

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

**Register slash command.** Register a workspace slash command handler (http URL or MCP tool name).

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**List slash commands.** List registered slash commands in a workspace.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Register FSM hook.** Register an FSM hook invoked on matching thread state transitions.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**List FSM hooks.** List registered FSM automation hooks in a workspace.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Get thread context.** Pack thread messages, edits, references, FSM history, and the workspace glossary for agent prompts. Edits are lean by default (id/editor/timestamp only); pass include_edits=true for full before/after bodies. The glossary (canonical term definitions) is included by default when non-empty; pass include_glossary=false to drop it. Pass as_of=<event_id> to replay the thread as it stood at that event-log id (deterministic over the immutable log; audit / re-ask from before a tangent). Pass token_budget=<n> to cap the message page by estimated tokens: the opening message and the recent tail are kept, the middle is folded into an auditable elision marker in fixed blocks. The result is two text parts: the stable prefix (workspace boot, brief, messages) and the volatile tail (state, lease, cursors, prefix sha256). Those strings are the bytes REST returns with split=true.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "as_of": {
      "description": "Event-log id: reconstruct the thread as it stood at that point (as-of replay). Omit for the live pack.",
      "type": "integer"
    },
    "delta": {
      "default": false,
      "description": "Return a delta instead of the whole pack. The first content part is always the delta head (delta, prefix_unchanged, prefix_sha256, prefix_bytes), never empty; the second is the volatile tail. When since_prefix_sha matches the current prefix, the head has prefix_unchanged true and no messages or prefix. With message_cursor, the head adds the messages after that cursor when appending them rebuilds the prefix; otherwise it carries the replacement prefix.",
      "type": "boolean"
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
    "max_bytes": {
      "description": "Cap the canonical pack by bytes. Elision grows by fixed message blocks until the pack fits or only the opener and newest message remain.",
      "minimum": 1,
      "type": "integer"
    },
    "message_cursor": {
      "description": "Page messages after this id. With delta, it is also the delta cursor.",
      "format": "uuid",
      "type": "string"
    },
    "message_limit": {
      "maximum": 500,
      "minimum": 1,
      "type": "integer"
    },
    "since_prefix_sha": {
      "description": "Hex sha256 of a prefix the caller already holds. Requests a delta on its own.",
      "type": "string"
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

**Snapshot thread context.** Freeze the assembled context pack (live or as_of) into the content-addressed artifact store — a tamper-evident, deduped record of exactly what the agent was handed. Same params as get_thread_context; returns the artifact (kind=context_snapshot). Requires artifact:upload. Fetch the bytes via the artifact sha.

**Capability:** `artifact:upload`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**Get workspace context.** Pack workspace channels, thread contexts (bounded by thread_limit), and the workspace glossary (once at the top level).

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "include_glossary": {
      "default": true,
      "description": "Include the workspace glossary once at the top level (grounding); omitted when empty. Set false to drop it.",
      "type": "boolean"
    },
    "max_bytes": {
      "description": "Byte cap applied to each nested thread pack.",
      "minimum": 1,
      "type": "integer"
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

**Request approval.** Human-in-the-loop gate: open a durable approval gate and return {status: input_required, gate_id} without blocking. A human resolves it later (accept/decline/cancel) over the /ui; poll get_approval_gate for the outcome. Silence is never consent. Pass thread_id to make it a claim gate — while the gate is pending, claim_next will not hand that thread to an agent.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

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

**Get approval gate.** Poll a durable approval gate by id. Returns the gate — state is pending until a human answers, then accepted/declined/cancelled with any content they supplied — or null if no such gate exists in your workspace.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

**Link Slack channel.** Link a Slack channel to a Maidan thread so the projector bridges messages both ways. The workspace/channel and attribution member come from the authenticated caller and thread. Requires workspace:write + access to the thread.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**List Slack channel links.** List the Slack channel links in your workspace. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### `unlink_slack_channel`

**Unlink Slack channel.** Remove a Slack channel link in your workspace. Returns {unlinked: bool} (false if no such link belongs to your workspace). Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**Link GitHub issue.** Link a GitHub issue/PR to a Maidan thread so the projector bridges messages both ways. The workspace/channel and attribution member come from the authenticated caller and thread. Requires workspace:write + access to the thread.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

**List GitHub issue links.** List the GitHub issue/PR links in your workspace. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

### `unlink_github_issue`

**Unlink GitHub issue.** Remove a GitHub issue/PR link in your workspace. Returns {unlinked: bool} (false if no such link belongs to your workspace). Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

### `tombstone_message`

**Tombstone message.** Withdraw a message. Same store path as DELETE /messages/{id}: message:post, plus channel:admin when the caller is not the author.

**Capability:** `message:post`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

### `open_group_dm`

**Open group DM.** Open a group DM among at least three workspace members. Same store path as POST /workspaces/{wid}/group-dms. Requires workspace:read. This writes a conversation.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

```json
{
  "properties": {
    "member_ids": {
      "items": {
        "format": "uuid",
        "type": "string"
      },
      "type": "array"
    },
    "title": {
      "type": "string"
    },
    "workspace_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "workspace_id",
    "member_ids"
  ],
  "type": "object"
}
```

### `list_group_dms`

**List group DMs.** List group DMs for one member. member_id must be the caller. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `get_group_dm`

**Get group DM.** Fetch one group DM the caller participates in. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "group_dm_conversation_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "group_dm_conversation_id"
  ],
  "type": "object"
}
```

### `post_group_dm_message`

**Post group DM message.** Post a message into a group DM the caller participates in. Same store path as POST /group-dms/{id}/messages. Requires message:post.

**Capability:** `message:post`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: false`, `openWorldHint: false`

```json
{
  "properties": {
    "body": {
      "type": "string"
    },
    "group_dm_conversation_id": {
      "format": "uuid",
      "type": "string"
    },
    "metadata": {
      "type": "object"
    }
  },
  "required": [
    "group_dm_conversation_id",
    "body"
  ],
  "type": "object"
}
```

### `remove_thread_dependency`

**Remove thread dependency.** Remove one dependency edge: thread_id no longer depends on depends_on_thread_id. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "depends_on_thread_id": {
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
    "depends_on_thread_id"
  ],
  "type": "object"
}
```

### `list_thread_dependents`

**List thread dependents.** List threads that depend on this thread. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `clear_thread_lineage`

**Clear thread lineage.** Clear the run-lineage row for a thread. NotFound when no row exists. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

### `remove_member_skill`

**Remove member skill.** Remove a skill from a member. Governance skills follow the REST self-versus-admin split; routing tags are self-only. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

### `remove_thread_required_skill`

**Remove thread required skill.** Remove a required skill from a thread. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

### `delete_memory_block`

**Delete memory block.** Delete a memory block by id. A block in another workspace is NotFound. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "block_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "block_id"
  ],
  "type": "object"
}
```

### `delete_glossary_term`

**Delete glossary term.** Delete a glossary term in the caller workspace. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

### `delete_recipe`

**Delete recipe.** Delete a recipe. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
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

### `set_task_schedule_active`

**Set task schedule active.** Pause or resume a task schedule. Same store path as PUT /task-schedules/{id}. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "active": {
      "type": "boolean"
    },
    "schedule_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "schedule_id",
    "active"
  ],
  "type": "object"
}
```

### `delete_task_schedule`

**Delete task schedule.** Delete a task schedule. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "schedule_id": {
      "format": "uuid",
      "type": "string"
    }
  },
  "required": [
    "schedule_id"
  ],
  "type": "object"
}
```

### `revoke_slash_command`

**Revoke slash command.** Revoke a slash command. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "command_id": {
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
    "command_id"
  ],
  "type": "object"
}
```

### `revoke_fsm_hook`

**Revoke FSM hook.** Revoke an FSM hook. Requires workspace:write.

**Capability:** `workspace:write`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "hook_id": {
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
    "hook_id"
  ],
  "type": "object"
}
```

### `clear_review_requirement`

**Clear review requirement.** Clear a thread review requirement. Requires channel:admin.

**Capability:** `channel:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

### `get_review_requirement`

**Get review requirement.** Read the review requirement on a thread. NotFound when unset. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `list_reviewers`

**List reviewers.** List reviewers on a thread. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `remove_reviewer`

**Remove reviewer.** Remove a reviewer from a thread. Requires channel:admin.

**Capability:** `channel:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

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

### `list_votes`

**List votes.** List votes on a message. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `list_message_edits`

**List message edits.** List the edit history of a message. A tombstoned message returns an empty list unless the caller bypasses access checks. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "limit": {
      "default": 100,
      "maximum": 500,
      "minimum": 1,
      "type": "integer"
    },
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

### `mark_all_notifications_read`

**Mark all notifications read.** Mark every notification for this member read. Same store path as POST /members/{id}/notifications/read-all, named beside mark_notification_read. Returns {cleared}. member_id must be the caller. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: false`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `advise_land_gate`

**Advise land gate.** Ask the configured advisor for a land-gate recommendation. Read-only: it does not write the gate or the requirement. NotFound when no advisor is configured. Requires thread:transition.

**Capability:** `thread:transition`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: true`

```json
{
  "properties": {
    "instructions": {
      "type": "string"
    },
    "state": {
      "type": "object"
    },
    "thread_id": {
      "format": "uuid",
      "type": "string"
    },
    "thresholds": {
      "type": "object"
    }
  },
  "required": [
    "thread_id",
    "state"
  ],
  "type": "object"
}
```

### `create_secret`

**Create secret.** Store a named secret in the caller workspace. Returns metadata only; the plaintext is not echoed. Requires secret:admin.

**Capability:** `secret:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "name": {
      "type": "string"
    },
    "value": {
      "type": "string"
    }
  },
  "required": [
    "name",
    "value"
  ],
  "type": "object"
}
```

### `delete_secret`

**Delete secret.** Delete a named secret in the caller workspace. Requires secret:admin.

**Capability:** `secret:admin`

**Hints:** `readOnlyHint: false`, `destructiveHint: true`, `idempotentHint: true`, `openWorldHint: false`

```json
{
  "properties": {
    "name": {
      "type": "string"
    }
  },
  "required": [
    "name"
  ],
  "type": "object"
}
```

### `get_artifact`

**Get artifact.** Return artifact metadata and content_base64 bytes for a sha256 the caller workspace can access. A missing access ref is NotFound. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `list_members`

**List members.** List members of a workspace. Same store path as GET /workspaces/{wid}/members. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

### `get_member`

**Get member.** Fetch one member in the caller workspace. Another workspace is NotFound, same as an unknown id. Requires workspace:read.

**Capability:** `workspace:read`

**Hints:** `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`, `openWorldHint: false`

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

## Resources

### `workspace` — `maidan://workspaces/{id}`

Workspace metadata.

### `channel` — `maidan://channels/{id}`

Channel metadata.

### `boot` — `maidan://boots/{channel_id}`

Workspace boot for a channel: the bytes a thread pack prefix starts with.

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

