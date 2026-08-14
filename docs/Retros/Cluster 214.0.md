# Cluster 214.0 retro — three writes, one tx, and the domain migration closes

> Tag **`v214.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 13.

## What shipped

- References + artifacts migrated to the transactional outbox — the last domain
  mutations. `add_reference_with_event` (`ReferenceAdded`) and
  `upsert_artifact_with_event(new, ref_workspace)` (upsert + Cluster-204 access ref
  + `ArtifactUpserted`, all in one tx).

## Surprises / decisions

- **The artifact upload was the widest fold in the whole migration.** Every other
  `*_with_event` folded *one* domain write + its event. The artifact upload route
  did *three* dependent writes — `upsert_artifact`, a conditional
  `record_artifact_ref` (the Cluster-204 per-workspace access link), and
  `publish(ArtifactUpserted)`. Folding just the upsert + event would have left the
  event committed *before* the access ref (a brief window where a reactor to
  `ArtifactUpserted` could 404 on the blob). So the method takes `ref_workspace:
  Option<WorkspaceId>` and does upsert → ref → event **atomically**, preserving the
  original ordering. The conditional (`Some` for non-bypass) moved from the route
  into the store as a parameter — the route computes
  `(!auth.bypass).then_some(auth.workspace_id)`.
- **Cluster-204 isolation is *strengthened*, not just preserved.** Previously the
  access ref was a separate write after the upsert; a crash between left a blob
  with no ref (unreadable, or an orphaned upsert). Now they commit together — the
  ref exists iff the artifact upsert did.
- **References were the easy scope-less case again.** `ReferenceAdded` carries the
  whole reference, so `add_reference_with_event` needs no resolver — the same shape
  as the creation events (213). Factoring the shared unique-violation → `Conflict`
  mapping into `map_ref_err` kept the two variants tidy.

## Capability table extension

| Change | Where |
|--------|-------|
| Reference + artifact transactional outbox (`add_reference_with_event`; `upsert_artifact_with_event` + `record_ref_in_tx`) | `store/*/{refs,artifacts}.rs`, `routes/{reference,artifact}.rs` |

## Risks identified + still open

- **The domain-mutation outbox migration is COMPLETE (205–214)** — every event
  tied to a domain-table write now commits atomically with it. Nothing open here.

## A note on `publish()` — it stays, and that's correct

I expected 214 to leave `publish()` with a single caller (the relay) to be
renamed. A grep says otherwise: two callers remain, both legitimate, both
appending **standalone events** with no domain-table row to be atomic with —
(1) the federation **relay** (`federation.rs`, re-publishing *remote* events onto
the local bus) and (2) `publish_routed_mentions` (fanning a durable
`MentionRecorded` to each *auto-parsed* @mention for realtime routing — distinct
from the explicit-mention-API `record_mention_with_event` of 207; the parsed
@mentions have no `maidan_mentions` row). `publish()` *is* "durably append a
standalone event + notify the bus" — exactly what those two need. So there's no
rename cleanup: the refactor concludes at 214, and `publish()` remains a correct
primitive for events that aren't tied to a domain mutation.

## Forward look

The transactional-outbox refactor (205–214) is done. Program A's remaining items:
federation ingest trust policy + an RLS spike. Then Programs B (agentic
orchestration), C (notifications & reach), D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Concludes the
domain-mutation arc of the [[Retros/Cluster 205.0]] transactional-outbox refactor.
