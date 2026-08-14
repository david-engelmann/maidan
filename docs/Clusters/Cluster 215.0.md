# Cluster 215.0 — federation ingest trust policy

**Theme:** Program A (security & correctness round 2), part 14 — harden the
federated-event ingest surface with an event-kind allowlist and a nested-workspace
re-scope fix.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v215.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `EventKind::federatable()` allowlist (allowlist-by-default via exhaustive match; `ArtifactUpserted` excluded) | `maidan-types/src/events.rs` |
| Enforce the allowlist on ingest (push endpoint + pull worker both route through `ingest_envelope`) | `federation.rs` |
| Fix `MemberJoined` nested `member.workspace_id` remap leak | `federation.rs::remap_event_workspace` |

## Why

A federated peer authenticates and pushes (or serves, for our pull worker) a batch
of events that we append to the local event log. Two trust gaps:

1. **No event-kind allowlist.** Every `EventKind` was accepted verbatim. A peer
   could push any kind — including a future non-content kind added later — into the
   local log without review.
2. **Inconsistent workspace re-scoping.** `remap_event_workspace` re-scopes the
   event's top-level `workspace_id` to the *local* peer workspace, and re-scopes the
   nested `channel.workspace_id` for `ChannelCreated` — but **not** the nested
   `member.workspace_id` for `MemberJoined`. So a federated member carried the
   peer's *remote* workspace id in the local view.

## The change

- **`EventKind::federatable()`** — an **allowlist-by-default** predicate: an
  exhaustive `match` classifies every kind, so a new event kind fails to compile
  until consciously classified (a peer can't inject an unreviewed kind). All
  collaboration-content kinds are federatable; **`ArtifactUpserted` is not** —
  federation replicates *events*, not artifact *blobs*, so an ingested
  `ArtifactUpserted` would announce a `sha256` whose bytes never arrive (a dangling
  reference / existence oracle). `ingest_envelope` rejects non-federatable kinds
  with `403` (covering both the push endpoint and the pull worker, which share it).
- **`MemberJoined` remap** now sets `member.workspace_id = <local>` too — the same
  re-scope `ChannelCreated` already did — so no remote workspace id leaks into the
  local member view.

## Exit criteria

- A peer pushing `ArtifactUpserted` is rejected; a federated `MemberJoined` carries
  the local workspace id at every level — **met**.
- `v215.0.0` tagged.

## Verification & limits

- `events::federatable_allowlist_excludes_only_artifacts` (types): the allowlist is
  exactly the content kinds.
- `federation::remap_tests::member_joined_remap_rescopes_nested_workspace`: the
  nested member workspace is re-scoped.
- `federation_e2e::federation_ingest_rejects_non_federatable_artifact_event`: a
  pushed `ArtifactUpserted` batch → `403`.
- Behaviour-preserving: the existing `federation_e2e` ingest/dedupe/peer-list flow
  (content kinds) stays green.
- **Scope note:** the trust boundary is **ingest** (each side allowlists what it
  accepts). Egress (serving our events when peers pull) is unchanged — a peer
  running this allowlist rejects our `ArtifactUpserted` on its own ingest. **RLS
  spike** is Program A's remaining item.

## References

- [[Retros/Cluster 215.0]]; `maidan-types/src/events.rs`, `federation.rs`. Program:
  [[Roadmap]] + memory `maidan-next-arc-program` (Program A). Follows the completed
  transactional-outbox refactor (205–214).
