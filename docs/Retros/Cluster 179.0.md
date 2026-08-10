# Cluster 179.0 retro — A2A ingress channel/thread RBAC

> Tag **`v179.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc A (security & correctness), part 1. **Closes a real vuln.**

## What shipped

- `POST /a2a/v1/rpc` now enforces `ensure_channel_access` on both the write
  (`SendMessage` → `post_a2a_message`) and read (`tasks/get` →
  `ensure_task_workspace_access`) paths — the surface the 160–165 RBAC arc had
  missed. A non-member `message:post` token can no longer post into / read a
  private channel it isn't a member of.
- Backlog hygiene: corrected the stale `docs/Open Work.md` line that still called
  `reference.rs` "the last remaining RBAC surface" (Cluster 165 closed it), and
  recorded the A2A closure.

## Surprises

- **The vuln was a missing call, not missing infrastructure.** Both A2A handlers
  already had the resolved `AuthContext` and the thread context in hand — they
  simply never called the channel helper the rest of the codebase uses. One line
  per path. The gap existed because the A2A surface was built before the RBAC arc
  and never revisited when 160–165 swept the other surfaces.
- **The existing A2A e2e couldn't have caught it** — `a2a_protocol_e2e` runs on
  `AppState::for_tests` (auth bypassed), so the gate is short-circuited there.
  The regression test had to go in `channel_access_e2e` (auth enabled + real
  minted tokens + a private channel). Lesson: a surface's security posture is
  only tested where auth is actually on.

## Decisions

- **Reuse `ensure_channel_access`, not a bespoke check** — identical semantics
  across REST/MCP/WS/A2A; one place to reason about private-channel access.
- **Leave the `__dm__` exemption to Cluster 180** — tightening it uniformly
  across all surfaces (including this one) is its own cluster; scoping 179 to the
  private-channel hole keeps the fix surgical.

## Capability table extension

| Change | Where |
|--------|-------|
| A2A JSON-RPC ingress channel-gated (post + task-read) | `crates/maidan-server/src/a2a_agent.rs` |

## Risks identified + still open

- **Low (net risk-reducing).** Additive gate reusing a proven helper; bypass +
  public + member paths unchanged (existing bypass e2e green). The `__dm__`
  generic-route gap remains, tracked for Cluster 180.

## Forward look

Arc A continues: DM generic-route participant check (180), EventKind three-parser
parity guard (181), audit-log coverage (182), default-on rate limits + body cap
(183), dual-write atomicity (184).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
