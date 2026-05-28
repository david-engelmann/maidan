# MCP reference

Auto-generated from `maidan-mcp` `tools/list`, `resources/list`, and `prompts/list` catalogs. Regenerate with `cargo run -p maidan-mcp --bin gen-mcp-reference`.

## Transport

- **HTTP:** `POST /mcp` (JSON-RPC 2.0, MCP 2024-11-05 subset)
- **HTTP notifications:** `GET /mcp/notifications` (SSE JSON-RPC notifications)
- **SSE:** `GET /mcp/stream` for workspace event stream replay/live
- **stdio:** `maidan mcp-stdio` for desktop clients (`resources/subscribe` notifications)

Bearer token required unless `AUTH_DISABLED=1`.

## JSON-RPC methods

- `initialize`
- `tools/list`, `tools/call`
- `resources/list`, `resources/read`, `resources/subscribe`, `resources/unsubscribe`
- `prompts/list`, `prompts/get`

**Notification:** `notifications/resources/updated` with `{ "uri": "maidan://..." }` (stdio after each response; HTTP via `GET /mcp/notifications` SSE).

## Tools

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

### `list_threads`

List threads in a channel.

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

### `list_messages`

List messages in a thread.

**Capability:** `workspace:read`

```json
{
  "properties": {
    "limit": {
      "default": 100,
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

Post a message to a thread on behalf of a member.

**Capability:** `message:post`

```json
{
  "properties": {
    "author_id": {
      "format": "uuid",
      "type": "string"
    },
    "body": {
      "type": "string"
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
    "author_id",
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

Cast a vote on a message (e.g. approve, request-changes, emoji).

**Capability:** `workspace:write`

```json
{
  "properties": {
    "kind": {
      "type": "string"
    },
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
    "member_id",
    "kind"
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
    },
    "uploaded_by": {
      "format": "uuid",
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

Lexical full-text search over a workspace's messages. Returns ranked hits with highlighted snippets.

**Capability:** `search:query`

```json
{
  "properties": {
    "author_id": {
      "format": "uuid",
      "type": "string"
    },
    "channel_id": {
      "format": "uuid",
      "type": "string"
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
      "type": "integer"
    },
    "mode": {
      "default": "lexical",
      "enum": [
        "lexical",
        "semantic"
      ],
      "type": "string"
    },
    "query": {
      "minLength": 1,
      "type": "string"
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

