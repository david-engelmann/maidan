# Cluster 142.0 retro — Slash-command registry in the console

> Tag **`v142.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

- **`/ui/api` slash-command routes**: `GET /ui/api/workspaces/:wid/slash-commands`
  (read, `workspace:read`); `POST` (register) + `DELETE
  /ui/api/workspaces/:wid/slash-commands/:cid` (revoke) on the write router
  (`workspace:write`), reusing the tested
  `slash_commands::{list,create,revoke}_slash_command`.
- **"Slash" tab in `index.html`** (`panel-slash`): register a command
  (name / description / `handler_kind` http\|mcp_tool / `handler_target`), a
  refreshable list (kind, name, enabled/revoked, target) with per-row Revoke,
  and a one-time signing-secret display (copy button + "shown once" warning)
  for `http` handlers.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| n/a | An "execute" button | Slash commands have no execute endpoint; they dispatch when a `/name args` message is posted (already in the UI). The tab links users to that. |
| n/a | Edit-in-place | The backend has no update route — register + revoke is the lifecycle. |
| n/a | `/ui/api` slash backend test | Handlers + `/ui/api` middleware are each already covered. |

## Surprises

- **There is no "run a slash command" API.** Dispatch is a side effect of
  posting a message whose body starts with `/name`; the handler result is
  merged into the message metadata (`slash_command` + `slash_response`). So the
  UI surface is purely the **registry** — invocation rides the existing
  message-post path.
- **`http` handlers mint a one-time secret** (like API tokens) for webhook
  signing; `mcp_tool` handlers don't. The register response carries `secret`
  only for `http`, so the UI shows it once and only then.

## Decisions

- **A registry tab, not an invoker.** Matches the backend shape and avoids a
  redundant message-post UI.
- **Reuse handlers under `/ui/api`** (as with every UI cluster) — no new
  backend logic; secret handling mirrors the token-mint affordance.

## Capability table extension

| Capability | Where |
|------------|-------|
| Register / list / revoke slash commands in the `/ui` console | `static/index.html`, `/ui/api/workspaces/:wid/slash-commands[/:cid]` |

## Risks identified + still open

- **JS behavior inspection-verified** (no browser) — standing UI limit; the
  `ui_js_contract` guard covers references, the `slash_commands` e2e covers the
  API.

## Forward look

This surfaces the last unsurfaced backend collaboration feature. The `/ui`
console now covers messaging (channels/threads/reactions/pins/edits), DMs +
group DMs, presence, the operator console (deliveries/DLQ, audit, reindex),
tokens/apps, federation, and slash commands. Remaining UI work is polish /
new product surface rather than catching up to the backend.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
