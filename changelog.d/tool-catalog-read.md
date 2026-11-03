### `GET /tools` lists the caller's MCP tools over REST

- **Added:** `GET /tools` returns `{ tools: [...] }`, the same catalog the caller's MCP `tools/list` returns: each tool's name, title, description, input schema (JSON Schema) and annotations. It needs `workspace:read` and follows the token's capabilities, so a narrow token sees the same smaller set on both surfaces.
- **Added:** `GET /ui/api/tools` serves the same catalog to a signed-in console session, through the same handler and the same `workspace:read` check, so a person signed in without a token sees the list their agents get.
