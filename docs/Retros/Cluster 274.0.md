# Cluster 274.0 retro — launch positioning + review reconciliation

> Tag **`v274.0.0`**. Phase XXIV (post-gate hardening). **Docs/positioning:
> new pitch + folded a public-launch-readiness review into the backlog.** No new
> gate tag.

## What shipped

A second external agent produced a launch-readiness review of the released `v272.0.0`
(it actually *ran the binary*). This cluster acts on the verified, high-value parts.

- **New pitch — off "Slack for agents".** The maintainer chose a problem-first hook:
  *"AI agents are brilliant and forgetful. Maidan gives a team of them a shared, durable
  place to work."* Threaded through the README, `docs/Integration.md`,
  `docs/Architecture.md`, and the OpenAPI `info.description`. The framing now leads with
  the durable-shared-workspace story (transactional state+event, self-healing event log,
  capability scoping) instead of the chat-app analogy.
- **Fixed a genuinely broken README command.** The quickstart showed
  `AUTH_DISABLED=1 cargo run`, which **fails closed** since Cluster 157 (needs the
  explicit `MAIDAN_ALLOW_INSECURE_NO_AUTH=1` ack). Corrected + explained.
- **Refreshed stale docs.** `Architecture.md` said baseline `v179.0.0` (and `v268`);
  updated to `v273`. Relabeled the **A2A** endpoint as an *experimental Maidan subset*
  (README + Integration) — we never claimed "v1.0 compliant", but listing A2A flat
  alongside MCP/REST implied more maturity than it has. Added a "What Maidan is not"
  note (not an LLM runtime / orchestration planner / hosted SaaS).
- **Folded the review into Open Work** as a "Public-launch readiness" backlog table:
  version-truthfulness (`0.0.0`), the SQLite first-write `database is locked` finding,
  a one-command quickstart, `maidan init`, LangChain/AutoGen recipes + interop CI,
  published benchmark methodology, A2A v1.0 compliance (seeded by the review's gap
  matrix), architecture-doc split, and GitHub metadata.

## Surprises / decisions

- **Verified before acting.** A peer-agent review can be wrong (the last one asserted a
  `maidan.world` domain that didn't resolve). I checked each load-bearing claim against
  the current tree: the broken auth command (README:78), `version = "0.0.0"`
  (`Cargo.toml:20`), and the stale `Architecture.md` baseline are all **real**. The
  "we claim A2A v1.0" framing was **overstated** — we don't claim it; we just
  under-labeled A2A as more mature than a subset. Fixed the accurate version.
- **Didn't commit the review's kit as working code.** Its `Dockerfile.quickstart` /
  `compose.quickstart.yaml` / demo scripts are a good starting point, but even the
  reviewer didn't build the image. Committing untested deployment code as if it works
  would repeat the exact "looks right, isn't" trap — so the quickstart is a backlog
  item with the kit as reference, to be built + tested in its own cluster.
- **The review's best contribution is confidence, not new features.** It independently
  praised the same four technical stories we consider strongest (NOTIFY floor 258,
  sharded fan-out 201, LSN replica routing 261–266, typed IDs) — recorded in Open Work
  as the honest launch-narrative pillars.

## Capability table extension

| Change | Where |
|--------|-------|
| New pitch (problem-first hook) | `README.md`, `docs/Integration.md`, `docs/Architecture.md`, `openapi/mod.rs` |
| Fixed broken `AUTH_DISABLED` quickstart command | `README.md` |
| A2A relabeled experimental; "what Maidan is not" | `README.md`, `docs/Integration.md` |
| Architecture baseline `v179`→`v273` | `docs/Architecture.md` |
| Public-launch-readiness backlog | `docs/Open Work.md` |

## Risks identified + still open

- The launch-readiness items are now tracked, not done. The two highest-signal *code*
  items — **version truthfulness** and the **SQLite first-write lock** — deserve early
  attention (both are trivially visible to a first-time evaluator).

## Forward look

The launch backlog is in Open Work's "Public-launch readiness". Natural first cluster:
the version-truthfulness fix + a SQLite first-write regression test (both small,
both P0, both visible on the very first run).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 273.0]]. Launch-readiness review by a separate agent; verified and
reconciled here.
