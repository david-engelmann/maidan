# Handoff: Maidan Connected-App E2E — What We Have and What's Needed

Date: 2026-10-07 (updated 2026-10-08)
From: Harness work (Claude/ChatGPT integration focus)
To: Agent on a dev machine

## What was accomplished

### Harness (connected-app-harness/)
- 9-provider E2E harness, 54/54 tests green, selftest green.
- Full review battery done (red-team + two-axis + own review), all findings fixed.
- MCP preflight, ordered multi-tool assertions, client telemetry.
- `docs/connected-app-playbook.md`: auth tiers, security invariants, network lessons.
- `docs/claude-desktop-local.md`: local testing guide.
- `tools/https_proxy.py`: HTTPS terminator for localhost MCP.

### Live verification (via managed browser)
- **ChatGPT**: Dev mode ENABLED (Free plan — research doc was wrong about paywall).
  Plugin Creator tooling available.
- **Claude**: Custom connector flow fully documented. Auth modes: "Sign in now"
  (OAuth), "Sign in when needed", "No sign-in". URL must be `https://`.

### Server
- Local Maidan server runs with MCP (240 tools, no-auth mode).
- Tool names corrected: `search_messages` (not `search`), `request_approval`
  (not `approval`). Harness config aligned.
- HTTPS works via the proxy (`https://127.0.0.1:18443/mcp`).

## The task

**Run the Maidan MCP server locally and test it via Claude Desktop on a dev machine.**

The sandbox cannot do this (no inbound, MITM proxy, no Claude login). A dev
machine with Claude Desktop installed and logged in is the right environment.

See `MAIDAN_AGENT_PROMPT.md` for the full step-by-step prompt.

## Test plan

1. Build and run the Maidan server locally with no-auth for testing.
2. Configure Claude Desktop with the `maidan-local` MCP server (stdio or HTTP).
3. Seed a test workspace with content containing "onboarding".
4. In Claude Desktop, verify:
   - Tool discovery lists `search_messages`, `post_message`, `request_approval`
   - "Search Maidan for onboarding" calls `search_messages`
   - "Post to general" calls `post_message`
   - Multi-tool: search then post, in order
5. Report results with MCP log evidence.

## Files of interest

- `MAIDAN_AGENT_PROMPT.md`: the full agent prompt
- `config.yaml`: suite definitions, provider config.
- `harness/providers/claude.py`: connector setup automation.
- `harness/providers/chatgpt.py`: dev-mode plugin automation.
- `harness/probe.py`: MCP preflight + server log correlation.
- `docs/connected-app-playbook.md`: all learnings.
