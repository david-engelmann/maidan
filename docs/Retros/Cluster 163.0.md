# Cluster 163.0 retro — verified WS/MCP subscribe grants (RBAC part E)

> Tag **`v163.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Closes the private-channel event leak on the WS + MCP SSE surfaces.

## What shipped

- **`apply_subscribe_grants(state, auth, filter)`** verifies each asserted
  private-channel grant against `channel_is_member` and drops the ones the caller
  isn't in (public + `__dm__` pass; bypass keeps all). Dropped grants → the
  channel is denied and added to `private_channel_deny`.
- **`ws.rs`** resolves the caller's identity (bearer / session / bypass) *before*
  applying grants; **`mcp_stream.rs`** passes its `AuthContext` through.
- Unit test with auth enabled proving drop-then-honor.

## What was deferred / not covered

| Surface | Why |
|---------|-----|
| `reference.rs` | No workspace/access check at all; needs entity→channel resolution — next cluster. |
| `channel:admin` membership API | The management surface — final RBAC cluster. |

## Surprises

- **Auth was resolved too late.** The WS subscribe handler applied channel grants
  *before* it authenticated the caller — fine when grants were merely trusted,
  but the moment verification needs the member id, the ordering had to flip.
  Moving the bearer/session resolution above the grant call (and using
  `bypass` when auth is disabled) was the crux of the change.

## Decisions

- **Drop unverified grants rather than error.** A workspace-wide subscribe that
  happens to assert a stale private grant should still succeed for the channels
  the caller *can* see, not fail wholesale — so an unverified grant is silently
  dropped and the deny-set does the rest. A subscribe *targeted* at a private
  channel the caller can't access still errors (the existing channel/thread
  check against the now-verified grant set).
- **Same `__dm__` exemption** as the rest of RBAC — DM grants pass; DM privacy is
  a separate (pre-existing) concern.

## Capability table extension

| Capability | Where |
|------------|-------|
| Verified subscribe grants | `subscribe_grants.rs`, `ws.rs`, `mcp_stream.rs` |

## Risks identified + still open

- **Private-channel content is now closed on read/write (REST+MCP) and events
  (WS+MCP SSE).** Remaining RBAC gaps: `reference.rs` and the management API.
  Shipped during the GitHub Actions outage; re-run CI on `main` when it recovers.

## Forward look

Cluster 164 guards `reference.rs`; the final RBAC cluster adds the
`channel:admin` capability + the `/channels/:cid/members` REST + MCP management
API. Then the arc is complete and arc 2 (perf + CI/CD) begins.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
