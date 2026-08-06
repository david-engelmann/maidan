# Cluster 165.0 — reference authorization (RBAC arc complete)

**Theme:** Close the last RBAC surface — references had no workspace or channel
check at all.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v165.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Gate `create_reference` / `list_references` on the referenced entity's channel access | `crates/maidan-server/src/routes/reference.rs` |
| Gate MCP `add_reference` (pre-dispatch) on both ref sides | `crates/maidan-mcp/src/tools/mod.rs` |

## Why

`RefSide` is `Thread | Message`, and the reference handlers had **no**
`ensure_workspace` and no access check — a token could create or list references
into any thread/message in the DB, including private channels and *other
tenants*. Now each side resolves via `ensure_thread_access` /
`ensure_message_access` (which also enforce the workspace), so a reference can
only touch entities the caller may access.

## Non-goals

- None — this completes the channel/thread RBAC arc (159–165).

## Exit criteria

- A non-member is denied `list_references` into a private thread; the RBAC arc is
  complete; suites green — **met**.
- `v165.0.0` tagged.

## Verification & limits

- `channel_access_e2e` gains a reference-denied assertion for a non-member.
  Matrix + MCP suites green (references still 403 at the capability check for a
  cap-less token; the new access check adds channel gating on top).
- **CI note:** GitHub Actions outage — validated locally; re-run CI on `main`
  when recovered.

## RBAC arc summary (159–165)

Private-channel access is enforced on **read/write** (REST 160, MCP 161–162),
**events** (WS/MCP subscribe 163), **management** (`channel:admin` 164), and
**references** (165), over a `channel_members` model (159). Public + `__dm__`
unchanged; Postgres RLS deferred.

## References

- [[Retros/Cluster 165.0]]; [[Clusters/Cluster 159.0]]–[[Clusters/Cluster 164.0]];
  `routes/reference.rs`, `tools/mod.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program`.
