# Cluster 214.0 — transactional outbox: references + artifacts (last domain mutations)

**Theme:** Program A (security & correctness round 2), part 13 — migrate the
reference and artifact mutations to the transactional-outbox pattern, closing the
domain-mutation migration.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v214.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `add_reference_with_event` (`ReferenceAdded`) | `store/{postgres,sqlite}/refs.rs`, `store.rs`, `*/mod.rs`, `routes/reference.rs` |
| `upsert_artifact_with_event(new, ref_workspace)` (`ArtifactUpserted` + Cluster-204 ref, all in one tx) + `record_ref_in_tx` | `store/{postgres,sqlite}/artifacts.rs`, `store.rs`, `*/mod.rs`, `routes/artifact.rs` |

## Why

References and artifacts are the last `publish()` domain-mutation callers. After
this cluster, `publish()`'s only remaining caller is the federation **relay**
(`federation.rs`), which re-publishes *remote* events (not a local write).

## The change

- **References** gain a scope-less `add_reference_with_event` (like the creation
  events — `ReferenceAdded` carries the whole reference, no resolver).
- **Artifacts** are the interesting one: the upload route did **three** writes —
  `upsert_artifact`, then (non-bypass) `record_artifact_ref` (the Cluster-204
  per-workspace access link), then `publish(ArtifactUpserted)`. The new
  `upsert_artifact_with_event(new, ref_workspace)` folds all three into **one
  transaction** (via a new `record_ref_in_tx`), preserving the upsert → ref → event
  ordering atomically. `ref_workspace` is `Some(auth.workspace_id)` for a
  non-bypass caller, `None` otherwise (the route computes
  `(!auth.bypass).then_some(auth.workspace_id)`). Both the single-shot and
  multipart upload routes converge on it.

## Exit criteria

- A reference add / artifact upload and its event (and the artifact's access ref)
  commit atomically — **met**.
- `v214.0.0` tagged.

## Verification & limits

- `event_log::reference_and_artifact_with_event_append_atomically` (store): a
  reference emits a durable `ReferenceAdded`; an artifact upload with
  `ref_workspace = Some` emits `ArtifactUpserted` **and** records the access ref in
  the same tx; with `None`, the event but no ref.
- Behaviour-preserving: `artifact_e2e`, `artifact_isolation_e2e` (the Cluster-204
  isolation still holds — the ref is written, just now in-tx), `http_crud_e2e`,
  `channel_access_e2e`, `event_emission_e2e` + the store suite (both backends) green.
- **Milestone:** the **domain-mutation** outbox migration is **complete** (205–214)
  — every event tied to a domain-table write commits atomically with it.
  `publish()` correctly **remains** for its two callers that append *standalone*
  events (no domain-table row to be atomic with): the federation relay and
  `publish_routed_mentions` (realtime `MentionRecorded` fan-out). No cleanup cluster
  is needed.

## References

- [[Retros/Cluster 214.0]]; `store/*/{refs,artifacts}.rs`, `routes/{reference,artifact}.rs`.
  Program: [[Roadmap]] + memory `maidan-next-arc-program` (Program A). Continues
  [[Retros/Cluster 213.0]].
