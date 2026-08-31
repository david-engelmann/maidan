# Cluster 341.0 retro — docs accuracy reconciliation (audit P2)

> Tag **`v341.0.0`**. Phase XXIV (post-gate hardening). **Cluster 10 of the post-flagship audit
> program.** Docs-only. No new gate tag.

## What shipped

The audit's P2 accuracy fixes — where the docs over- or under-stated what actually ships. Each
was verified against ground-truth code before editing.

- **A2A gRPC — reconciled to the honest "partial".** Three docs disagreed: `Claims.md` was right
  (gRPC exposes `get_task`/`cancel_task`/`list_tasks` only), `Architecture.md` implied full
  three-transport parity, and `Protocols.md` said "No gRPC binding" (also wrong — there *is* a
  partial one). Verified against `crates/maidan-server/src/a2a_grpc/mod.rs`, whose `A2AService`
  implements exactly those three methods. `Architecture.md` (intro, subsystem table, federation
  narrative) and `Protocols.md` now state: JSON-RPC + REST complete; gRPC partial (task
  read/cancel/list; send/push/streaming over JSON-RPC/REST).
- **Tool-count drift 78 → 85.** The catalog is 85 tools (`contracts/mcp-tool-names.json`); the
  live integrator docs still said ~78. Fixed in `docs/Framework Integrations.md`,
  `examples/README.md`, and the two current-count claims in `docs/Adoption.md`.
- **Dead GitHub link.** `Architecture.md` linked `[Capability Map](Capability-Map.md)` — the repo
  file is `Capability Map.md` (space), so the bare-hyphen form 404s on GitHub. Switched to the
  `Capability%20Map.md` form the other docs use (which `book/sync-docs.sh` rewrites to the
  hyphenated book URL, so the published link is unaffected).
- **README image pin `v315.0.0` → `v339.0.0`** — a recent released tag for the "pin a tag, not
  `:latest`" example.

## Surprises / decisions

- **Both directions of drift existed.** The gRPC docs were simultaneously too generous
  (Architecture) and too stingy (Protocols) — `Claims.md`, kept deliberately falsifiable, was the
  arbiter. Ground-truth-first: the fix was to read the service impl, not to average the docs.
- **Historical retro entries left alone.** `Capabilities.md`'s "78 tools" appears in the
  point-in-time cluster records where it was accurate; only *current-state* claims were updated.
  The internal strategy docs (`Undeniable*`, `Expansion Bets`) are point-in-time artifacts and
  were left as-is.

## Test evidence

Docs-only. `mdbook build` (with the linkcheck gate) clean after the edits, including the
`Capability%20Map.md` link and the gRPC rewrites.

## Forward look

Remaining audit items: **P1.5** (egress wire-path tests + LSN replica CI — test-confidence on
shipped launch features) and the P2 code-side items (Integration.md flagship-surface examples,
projector link-management surface, notification-router fan-out, Store trait split, unbounded
`list_threads` pagination).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
