# Cluster 143.0 — Richer message rendering (timestamps + inline slash results)

**Theme:** UI polish for the core thread view — surface data that's already on
each message but wasn't rendered: the post timestamp and slash-command results.
The UI/backend are now in parity (after 142), so this is the first
polish-not-catch-up cluster.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v143.0.0`**, no new gate tag.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Backend** | None — UI-only. |
| **UI (index.html `renderMessages`)** | Show `posted_at` (trimmed) in the meta line; render a compact slash-result block from `slash_command`/`slash_response` metadata (status + response). |

## Why

Both pieces were already in the message payload (`posted_at`; `metadata` with
`slash_command`/`slash_response` written by slash dispatch) but the message
view only showed `id · author · (edited)` + body + reactions. Rendering them
completes the slash loop (register in the Slash tab → post `/name` → see the
result in the thread) and gives messages a visible time.

## Non-goals

- Relative/"x minutes ago" timestamps — a trimmed ISO string is enough and
  dependency-free.
- Rendering arbitrary metadata — only the known `slash_*` keys (and the
  existing artifact keys) are surfaced.

## PR ladder (actual)

| # | Title |
|---|--------|
| 143.0.1 | `feat(ui): richer message rendering — timestamps + inline slash results` (#376) |
| 143.0.retro | `docs(retro): Cluster 143.0 + v143.0.0 tag prep` |

## Exit criteria

- Messages show a timestamp; slash results render inline when present; guard
  green — **met**.
- `v143.0.0` tagged after retro.

## Verification & limits

- `ui_js_contract` guard validates the new JS; no Rust change. Per the UI
  track's standing limit, JS *behavior* is inspection-verified (no browser);
  the `slash_commands` e2e covers the metadata shape.

## References

- [[Retros/Cluster 143.0]]; `static/index.html` (`renderMessages`,
  `renderSlashResult`).
