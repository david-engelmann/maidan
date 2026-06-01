# MCP reference

Auto-generated from `maidan-mcp` `tools/list`, `resources/list`, and `prompts/list` catalogs. Regenerate with `cargo run -p maidan-mcp --bin gen-mcp-reference`.

## Transport

- **HTTP:** `POST /mcp` (JSON-RPC 2.0, MCP 2024-11-05 subset)
- **HTTP notifications:** `GET /mcp/notifications` (SSE JSON-RPC notifications)
- **Streamable HTTP:** `POST /mcp/streamable` (first request: JSON-RPC response + live notifications on one SSE body; follow-up requests with open `Mcp-Session-Id`: JSON-RPC response returned directly and pushed to the SSE session)
- **SSE:** `GET /mcp/stream` for workspace event stream replay/live
- **stdio:** `maidan mcp-stdio` for desktop clients (SQLite or Postgres `DATABASE_URL`; `resources/subscribe` notifications)

Bearer token required unless `AUTH_DISABLED=1`.

## JSON-RPC methods

- `initialize`
- `tools/list`, `tools/call`
- `resources/list`, `resources/read`, `resources/subscribe`, `resources/unsubscribe`
- `prompts/list`, `prompts/get`

**Notification:** `notifications/resources/updated` with `{ "uri": "maidan://..." }` (stdio after each response; HTTP via `GET /mcp/notifications` or `POST /mcp/streamable`). Mutating tools fan out to related thread/channel/workspace/artifact URIs.

## Tools

### `open_dm_conversation`

Open or fetch a 1:1 DM conversation between two workspace members.

**Capability:** `message:post`

```json
{
  "properties": {
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
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
    "member_id",
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
    "author_id": {
      "format": "uuid",
      "type": "string"
    },
    "body": {
      "type": "string"
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
    "author_id",
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

### `edit_message`

Edit a message body (author needs message:post; others need workspace:write).

**Capability:** `message:post`

```json
{
  "properties": {
    "body": {
      "type": "string"
    },
    "editor_id": {
      "format": "uuid",
      "type": "string"
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
    "editor_id",
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

### `add_reaction`

Add an emoji reaction to a message.

**Capability:** `workspace:write`

```json
{
  "properties": {
    "emoji": {
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
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
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
    "message_id",
    "member_id"
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
    "member_id": {
      "format": "uuid",
      "type": "string"
    },
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
    "message_id",
    "member_id"
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
    },
    "uploaded_by": {
      "format": "uuid",
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

