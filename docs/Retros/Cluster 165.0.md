# Cluster 165.0 retro — reference authorization (RBAC arc complete)

> Tag **`v165.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> The final cluster of the channel/thread RBAC arc (159–165).

## What shipped

- **`create_reference` / `list_references`** (REST) resolve each Thread/Message
  side via `ensure_thread_access` / `ensure_message_access` — closing a path that
  had **no** workspace or channel check at all.
- **MCP `add_reference`** gated in the pre-dispatch gate (both `src` and `dst`
  sides, decoded via `RefSide`).
- `channel_access_e2e` reference-denied assertion.

## What was deferred / not covered

| Item | Why |
|------|-----|
| Postgres RLS | Deferred at the arc's start — app-layer enforcement (159–165) closes the reported vulnerability; RLS is defense-in-depth needing a per-connection GUC refactor. |
| Per-channel `channel:admin` scoping | A future refinement (164 retro). |

## Surprises

- **`RefSide` is only `Thread | Message`.** I'd braced for arbitrary
  entity→channel resolution (channels, artifacts, workspaces); the two-variant
  enum meant the existing `ensure_thread/message_access` helpers covered both
  sides directly — a five-line fix that also happened to close a real *cross-
  tenant* hole (no `ensure_workspace` was present).

## Decisions

- **Reuse the same helpers everywhere.** References, like every other content
  surface in the arc, defer to `ensure_*_access` — one enforcement definition,
  applied uniformly, so there's a single place to audit and evolve the policy.

## Capability table extension

| Capability | Where |
|------------|-------|
| Reference authorization | `routes/reference.rs`, `tools/mod.rs` |

## Risks identified + still open

- **The channel/thread RBAC arc is complete** — the #1 enterprise-readiness
  finding is closed across read/write, events, management, and references.
  Residual: Postgres RLS (defense-in-depth, deferred) and DM-via-generic-route
  (pre-existing, documented). **These clusters (160–165) were admin-merged during
  a GitHub Actions outage with local validation — re-run CI on `main` when GitHub
  recovers.**

## Forward look — Arc 1 done, Arc 2 next

Enterprise hardening (arc 1: 156–165) is complete. **Arc 2 (perf + CI/CD)**
begins: the localized DB fixes (H1 webhook full-table scan → workspace filter,
H6 model cache, R3 SQLite pragmas in `after_connect`, H4 outbox JOIN, R2
rate-limiter leak, H2 cursor coalesce) and the CI speedups (native arm64 release
runner, build-once smoke image, gha cache, the deferred trivy scan).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
