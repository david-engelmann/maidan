# Handoff: Maidan Connected-App E2E — What We Have and What's Needed

Date: 2026-10-07
From: Harness work (Claude/ChatGPT integration focus)
To: Main Maidan agent

## What was accomplished

### Harness (~/workspace/maidan-connected-app-harness/)
- 9-provider E2E harness, 54/54 tests green, selftest green.
- Full review battery done (red-team + two-axis + own review), all findings fixed.
- MCP preflight, ordered multi-tool assertions, client telemetry.
- `docs/connected-app-playbook.md`: auth tiers, security invariants, network lessons.
- `docs/claude-desktop-local.md`: local testing guide.
- `tools/https_proxy.py`: HTTPS terminator for localhost MCP.

### Live verification (via managed browser, David's accounts)
- **ChatGPT**: Dev mode ENABLED (Free plan — research doc was wrong about paywall).
  Plugin Creator tooling available.
- **Claude**: Custom connector flow fully documented. `maidan-test` connector
  was added with placeholder URL then removed (couldn't save unreachable URL).
  Auth modes: "Sign in now" (OAuth), "Sign in when needed", "No sign-in".
  URL must be `https://`, reachability checked, "Continue anyway" available
  but final save blocked if unreachable.

### Server
- Local Maidan server runs with MCP (240 tools, no-auth mode).
- Tool names corrected: `search_messages` (not `search`), `request_approval`
  (not `approval`). Harness config aligned.
- HTTPS works via the proxy (`https://127.0.0.1:18443/mcp`).

## The blocker

Claude/ChatGPT backends must reach the MCP server via public HTTPS.
The sandbox has no inbound connectivity; tunnels blocked by MITM proxy.
Claude Desktop in sandbox can't trust the proxy CA (read-only NSS) and the
app blocks cert-bypass flags.

## What the Maidan agent needs to do

**Deploy Maidan to a publicly reachable HTTPS URL with MCP enabled.**

Requirements for the E2E:
1. Public HTTPS URL (e.g. `https://maidan-dev.example.com`).
2. MCP endpoint at `/mcp` (Streamable HTTP transport).
3. No-auth mode (or a test API key) — the harness tests read paths first.
4. The server must expose `search_messages`, `post_message`, `request_approval`
   (all present in main @ cb69ed80).
5. CORS must allow the MCP client origins (or be permissive for testing).

Once the URL is live, the harness can:
- Add the connector to Claude via the managed browser (automated).
- Enable the plugin in ChatGPT via the managed browser (automated).
- Run the full E2E suites with server-side tool-call verification.

## Test plan once URL is live

1. `harness setup claude` → add connector with the public URL, No sign-in.
2. `harness test claude --suite reads` → prompt "Search Maidan for onboarding",
   verify `search_messages` called via server log.
3. `harness test claude --suite multi_tool` → verify ordered `search_messages`
   then `post_message`.
4. Same for ChatGPT via dev-mode plugin.

## Files of interest

- `config.yaml`: suite definitions, provider config.
- `harness/providers/claude.py`: connector setup automation.
- `harness/providers/chatgpt.py`: dev-mode plugin automation.
- `harness/probe.py`: MCP preflight + server log correlation.
- `docs/connected-app-playbook.md`: all learnings.
