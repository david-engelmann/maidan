# Cluster 142.0 — Slash-command registry in the console

**Theme:** Surface the workspace slash-command registry in the `/ui` — a new
"Slash" tab to register, list, and revoke commands. The last unsurfaced backend
collaboration feature.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v142.0.0`**, no new gate tag.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Routes (app.rs)** | `GET /ui/api/workspaces/:wid/slash-commands` (read, `workspace:read`); `POST` (register, `workspace:write`) + `DELETE /ui/api/workspaces/:wid/slash-commands/:cid` (revoke, `workspace:write`) on the write router. Reuses `slash_commands::{list,create,revoke}_slash_command`. |
| **UI (index.html)** | A `panel-slash` view: register (name / description / `handler_kind` http\|mcp_tool / `handler_target`), list with per-row Revoke, one-time signing-secret display for `http` handlers. |

## Why a registry, not an invoker

Slash commands have **no execute endpoint** — they dispatch implicitly when a
`/name args` message is posted to a thread (the result is merged into the
message metadata). Posting messages is already in the UI (FSM tab / channel
threads), so this cluster surfaces only the **registration** lifecycle.

## Non-goals

- An "execute" button — invocation is message-triggered; the tab points users
  to post `/name args` instead.
- Editing a command in place — register + revoke is the lifecycle (matches the
  backend, which has no update route).
- A dedicated `/ui/api` slash backend test — handlers + `/ui/api` middleware
  are each already covered.

## PR ladder (actual)

| # | Title |
|---|--------|
| 142.0.1 | `feat(ui): slash-command registry in the console` (#374) |
| 142.0.retro | `docs(retro): Cluster 142.0 + v142.0.0 tag prep` |

## Exit criteria

- Register / list / revoke in the UI; one-time secret surfaced; routes wired
  under `/ui/api`; guard green — **met**.
- `v142.0.0` tagged after retro.

## Verification & limits

- `ui_js_contract` guard validates the new JS; `fmt`/`clippy` clean. Per the UI
  track's standing limit, JS *behavior* is inspection-verified (no browser).

## References

- [[Retros/Cluster 142.0]]; `static/index.html`
  (`loadSlashCommands`/`registerSlashCommand`/`revokeSlashCommand`), `app.rs`,
  `slash_commands.rs`.
