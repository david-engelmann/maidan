# Cluster 160.0 retro — private-channel access control over REST (RBAC part B)

> Tag **`v160.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> The enforcement flip of the flagship channel/thread RBAC arc.

## What shipped

- **`maidan-auth/src/access.rs`** — `ensure_channel_access` (bypass=skip;
  public=workspace-open; private=`channel_members`; `__dm__` exempt), plus
  `ensure_thread_access`, `ensure_message_access`, and the bool `can_access_channel`.
- **Enforced on every REST content surface** — channels (get/list; create
  auto-adds the creator as admin of a new private channel), threads
  (create/list/get/context/transition), messages (post/list/get/edit/tombstone/
  purge/mention/edits), reactions/pins/votes, workspace-search hits, and the
  workspace-context pack.
- **`channel_access_e2e`** (auth enabled) proving the whole matrix.

## What was deferred / not covered

| Surface | Why deferred |
|---------|--------------|
| MCP tool enforcement (161) | The MCP content handlers don't receive `auth` — needs threading; kept out of the REST cluster for reviewability. |
| WS subscribe gate (`subscribe_grants`) | Still self-asserts membership; a distinct mechanism — private-channel *events* still leak to a non-member who asserts grants. |
| `reference.rs` | Has no ws/access check at all (pre-existing); needs entity→channel resolution. |
| DM via generic thread route | The `__dm__` exemption preserves pre-existing behavior; fixing needs a DM-participant check. |
| `channel:admin` management API | A later cluster. |

## Surprises

- **`for_tests` bypasses auth.** The default e2e harness runs with
  `auth_disabled = true`, so it injects `AuthContext::bypass()` and would *never*
  exercise enforcement (bypass=skip). The enforcement test had to build
  `AppState::new(..., false, ...)` (auth enabled) + mint real tokens, mirroring
  the capability-matrix harness — otherwise the tests would have passed while
  testing nothing.
- **The blast radius really was ~zero.** Every existing content test uses a
  public channel (or bypass), so the full suite stayed green; only the new
  auth-enabled test drives private channels.

## Decisions

- **One helper, called after `ensure_workspace`.** Every content handler already
  resolves a `ctx`/`chain` carrying `channel_id`, so the enforcement is a single
  uniform line — easy to audit for coverage and to extend to MCP.
- **`__dm__` exemption over per-DM logic in the channel helper.** Keeping DM
  membership in `dm.rs`/`group_dm.rs` avoids coupling the channel helper to the
  DM model; the (pre-existing) generic-route DM read gap is documented, not
  papered over.
- **Filter (not 403) for search + workspace-context.** A multi-channel read
  should return the visible subset, not fail wholesale — so hits/threads in
  inaccessible private channels are dropped, cached per channel to avoid N+1.

## Capability table extension

| Capability | Where |
|------------|-------|
| Private-channel access control (REST) | `maidan-auth/src/access.rs`, `routes/*.rs` |

## Risks identified + still open

- **Primary REST vuln closed; other surfaces still open** (MCP, WS events,
  references, DM-generic-route) — enumerated above and in Open Work. Shipped
  during a GitHub Actions outage, so **CI must be re-run on `main`** once GitHub
  recovers to confirm the full matrix in the canonical environment.

## Forward look

Cluster 161 threads `auth` into the MCP content handlers + `resources_read` and
calls the same helper — closing the MCP surface. Then the WS subscribe gate,
`reference.rs`, and the `channel:admin` membership API, before the arc moves on
to perf/CI (arc 2).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
