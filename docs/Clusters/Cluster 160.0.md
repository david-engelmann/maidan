# Cluster 160.0 — private-channel access control over REST (RBAC part B)

**Theme:** The enforcement flip of the flagship RBAC arc. Gate every REST content
surface on per-channel membership so a private channel is actually private.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v160.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `ensure_channel_access` / `ensure_thread_access` / `ensure_message_access` / `can_access_channel` | `crates/maidan-auth/src/access.rs` (new) + `lib.rs` |
| Enforce on channels (get/list; create auto-adds creator admin) | `routes/channel.rs` |
| Enforce on threads / messages / social | `routes/{thread,message,social}.rs` |
| Filter search hits + workspace-context threads by access | `routes/{search,workspace}.rs` |
| Enforcement e2e (auth enabled) | `tests/channel_access_e2e.rs` |

## Why

Authorization was workspace-flat: any `message:post` token could read or write
**any** channel/thread in its workspace, including private ones. This cluster
adds a single `ensure_channel_access(store, auth, channel_id)` — bypass callers
and public channels pass; a private, non-`__dm__` channel requires a
`channel_members` row — and calls it (or the thread/message variants) right after
`ensure_workspace` on every content handler. The creator of a new private
channel is auto-added as an admin so they don't lock themselves out.

## Semantics

- **Public** channel → open to the whole workspace (no rows).
- **Private** channel → only members (`channel_members`).
- **`__dm__`** system channel → exempt; DM/group-DM participant checks in
  `dm.rs`/`group_dm.rs` remain authoritative. Public channels and DMs are
  unchanged.

## Non-goals (follow-ups, tracked in Open Work)

- **MCP tool enforcement** — Cluster 161 (the MCP content handlers don't receive
  `auth` today; needs threading).
- **WS event-subscribe gate** — `subscribe_grants.rs` still trusts a client's
  asserted `channel_grants`; private-channel *events* still reach a non-member
  who asserts them.
- **`reference.rs`** — has no workspace/access check at all (pre-existing).
- **DM via the generic thread route** — the `__dm__` exemption preserves the
  pre-existing behavior that a DM thread is readable via `GET /threads/:id/...`;
  tighten by checking DM participants.
- **`channel:admin` + membership API** — a later cluster.
- **Postgres RLS** — deferred.

## Exit criteria

- Private-channel content is denied to non-members over REST; creator + explicit
  members allowed; public + DM unaffected; full suite green — **met**.
- `v160.0.0` tagged.

## Verification & limits

- `channel_access_e2e` (auth **enabled**): non-member denied post/list/get-thread/
  get-channel (403); creator auto-added; explicit member allowed; public channel
  open; DM thread readable (exemption). Full `cargo test -p maidan-server`
  (incl. testcontainers via local docker) green.
- **CI note:** shipped during a GitHub Actions **major outage** (action-download
  503s failing job setup) — validated locally and admin-merged per the user's
  authorization; re-run CI on `main` when GitHub recovers.

## References

- [[Retros/Cluster 160.0]]; scratchpad `rbac-plan.md`; `access.rs`,
  `routes/*.rs`. Program: [[Roadmap]] + memory `maidan-next-arc-program`.
