# Prompt for Agent: Claude Desktop E2E for Maidan Connected App

## Context

The connected-app E2E harness is ready at:
`https://github.com/david-engelmann/maidan-harness` (branch: main)

It contains:
- 9-provider harness (54/54 tests green)
- Claude/ChatGPT browser automation
- MCP preflight and verification
- `docs/connected-app-playbook.md`: all learnings
- `docs/claude-desktop-local.md`: local testing guide

## Your Task

Run the Maidan MCP server locally and test it via Claude Desktop.

### Step 1: Get the code

```bash
git clone https://github.com/david-engelmann/maidan-harness.git
cd maidan-harness
```

### Step 2: Build and run the Maidan server

You need the Maidan server binary. Either:
- Build from the maidan repo (main branch), OR
- Use the prebuilt binary if provided.

Run with no-auth for testing:
```bash
DATABASE_URL="sqlite:/tmp/maidan.db" \
MAIDAN_BIND="127.0.0.1:8080" \
MAIDAN_ALLOW_INSECURE_DEV_KEK=1 \
MAIDAN_SESSION_SECRET="dev-secret" \
MAIDAN_ALLOW_INSECURE_NO_AUTH=1 \
AUTH_DISABLED=1 \
./maidan-server
```

Verify: `curl http://127.0.0.1:8080/health` should return OK.

### Step 3: Configure Claude Desktop

Add to your Claude Desktop config:

**macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
**Linux**: `~/.config/Claude/claude_desktop_config.json`

```json
{
  "mcpServers": {
    "maidan-local": {
      "command": "/path/to/maidan-server",
      "args": [],
      "env": {
        "DATABASE_URL": "sqlite:/tmp/maidan.db",
        "MAIDAN_BIND": "127.0.0.1:8080",
        "MAIDAN_ALLOW_INSECURE_DEV_KEK": "1",
        "MAIDAN_SESSION_SECRET": "dev-secret",
        "MAIDAN_ALLOW_INSECURE_NO_AUTH": "1",
        "AUTH_DISABLED": "1"
      }
    }
  }
}
```

**IMPORTANT**: Claude Desktop spawns the server itself via stdio. The config
above uses stdio transport (command/args), NOT HTTP. The Maidan server must
support stdio MCP transport.

If Maidan only supports HTTP transport, use this instead:
```json
{
  "mcpServers": {
    "maidan-local": {
      "url": "http://127.0.0.1:8080/mcp"
    }
  }
}
```

Restart Claude Desktop after changing the config.

### Step 4: Seed test data

Create a workspace with:
- Name: `harness-test`
- Channel: `general`
- Messages containing the word "onboarding" (for search tests)

### Step 5: Run E2E tests

In Claude Desktop, send these prompts and verify:

1. **Tool discovery**: "What Maidan tools do you have available?"
   - Should list `search_messages`, `post_message`, `request_approval`, etc.

2. **Search**: "Search Maidan for the word onboarding"
   - Should call `search_messages` and return results.

3. **Post**: "Post 'hello from Claude Desktop' to the general channel"
   - Should call `post_message`.

4. **Multi-tool**: "Search for onboarding, then post a summary to general"
   - Should call `search_messages` THEN `post_message` in order.

### Step 6: Report results

For each test, report:
- ✅ PASS or ❌ FAIL
- The exact tool calls made (from Claude Desktop's MCP logs)
- Any errors

MCP logs:
**macOS**: `~/Library/Logs/Claude/mcp.log`
**Linux**: `~/.config/Claude/logs/mcp.log`

### Notes

- The harness (`harness/`) has automated suites, but they require browser
  automation. For now, manual testing via Claude Desktop UI is the goal.
- Tool names: `search_messages` (not `search`), `request_approval` (not `approval`).
- The server exposes 240 MCP tools. Focus on the messaging tools first.
- See `docs/connected-app-playbook.md` for auth tiers and security notes.
- See `docs/claude-desktop-local.md` for the HTTPS proxy setup (if needed
  for web-based testing later).
