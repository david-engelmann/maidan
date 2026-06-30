# Cluster 143.0 retro — Richer message rendering

> Tag **`v143.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

- **Timestamps in the thread view**: `renderMessages` now shows `posted_at`
  (trimmed to `YYYY-MM-DD HH:MM:SS`) in the meta line.
- **Inline slash-command results**: a new `renderSlashResult` renders a compact
  block from a message's `slash_command`/`slash_response` metadata — `⌘ /name
  args`, an ok / ✗ error / ⟳ retrying status, and the handler response
  (pretty-printed JSON or text).

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Future | Relative ("3m ago") timestamps | Trimmed ISO is enough and dependency-free. |
| n/a | Rendering arbitrary metadata | Only the known `slash_*` keys (and existing artifact keys) are surfaced. |

## Surprises

- **Nothing new on the wire.** Both additions were already in the message
  payload — `posted_at` and the `slash_*` metadata written by slash dispatch
  — just never rendered. The first cluster that's pure presentation.

## Decisions

- **Complete the slash loop in the view it happens in.** Slash results live in
  message metadata, so the thread view is where they belong — register (Slash
  tab, 142) → post `/name` → see the result here.
- **UI-only, no backend.** The data already existed; this is presentation.

## Capability table extension

| Capability | Where |
|------------|-------|
| Thread messages show timestamps + inline slash-command results | `static/index.html` (`renderMessages`/`renderSlashResult`) |

## Risks identified + still open

- **JS behavior inspection-verified** (no browser) — standing UI limit; the
  `ui_js_contract` guard covers references, the `slash_commands` e2e covers the
  metadata shape.

## Forward look

UI/backend are in parity; this was the first polish cluster. Further work is
optional presentation refinement or net-new product.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
