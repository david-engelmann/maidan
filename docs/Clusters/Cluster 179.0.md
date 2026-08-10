# Cluster 179.0 — security: A2A ingress channel/thread RBAC

**Theme:** Arc A (security & correctness), part 1 — close the one surface the
160–165 RBAC arc missed: the external A2A JSON-RPC ingress.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v179.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Enforce per-channel access on `POST /a2a/v1/rpc` write (`SendMessage`) + read (`tasks/get`) | `crates/maidan-server/src/a2a_agent.rs` |
| Backlog hygiene: correct the stale `reference.rs`/RBAC-surface line | `docs/Open Work.md` |

## Why

The channel/thread RBAC arc (Clusters 160–165) added `ensure_channel_access` to
every REST content route, MCP point-access + aggregate tools, WS/MCP subscribe
grants, and references — but **not** the A2A JSON-RPC surface. `post_a2a_message`
gated only on the `message:post` capability + `ensure_workspace`
(`a2a_agent.rs:210,228`), and the `tasks/get` read path
(`ensure_task_workspace_access`) checked only the workspace. So an external A2A
agent holding a workspace `message:post` token could **post into — and read tasks
whose context thread lives in — a private channel it isn't a member of**. This is
the exact confidentiality vuln 160–165 closed everywhere else, still open on a
live external ingress. A genuine, exploitable cross-channel hole.

## The fix

Both A2A handlers already receive the resolved `AuthContext` (`Extension(auth)`)
and resolve the target thread's context. The fix adds the missing check next to
the existing `ensure_workspace`:
- `post_a2a_message` → `ensure_channel_access(store, auth, thread_ctx.channel_id)`
  after `ensure_workspace`, before the post.
- `ensure_task_workspace_access` (the `tasks/get` gate) → the same check on the
  task's resolved context thread's channel.

Uses the same `maidan_auth::ensure_channel_access` as REST/MCP, so behavior is
identical across surfaces (bypass skips; public + `__dm__` open; private needs a
membership row).

## Non-goals

- The `__dm__` generic-route exemption (a DM thread readable via the generic
  route) is a *separate* gap closed uniformly across all surfaces in **Cluster
  180**; `ensure_channel_access` here inherits that exemption for now.

## Exit criteria

- A non-member `message:post` token is denied A2A post/read into a private
  channel; a member is allowed; suites green — **met**.
- `v179.0.0` tagged.

## Verification & limits

- `a2a_ingress_denies_non_members_in_private_channels` (channel_access_e2e, auth
  ENABLED): a non-member's `SendMessage` to a private-channel thread returns a
  JSON-RPC `error` (no message posted); the channel member succeeds. The existing
  `a2a_protocol_e2e` suite (bypass) stays green (bypass short-circuits the gate).
- Limit: enforcement is per-channel; the `tasks/get` gate resolves via the task's
  `context_id` thread — a task with no resolvable context thread falls back to
  the workspace check (unchanged).

## References

- [[Retros/Cluster 179.0]]; `crates/maidan-server/src/a2a_agent.rs`,
  `crates/maidan-auth/src/access.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program` (Arc A).
