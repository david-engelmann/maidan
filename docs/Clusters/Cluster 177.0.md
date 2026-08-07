# Cluster 177.0 — token: omit empty message metadata from the wire

**Theme:** Arc 4 (token round 3), part 3 — leaner message payloads (the
"omit-empty metadata" half of lean write-acks).

**Ladder:** Post-gate — **Phase XXIV**, tag **`v177.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `Message.metadata` is omitted from serialization when empty (`{}`/`null`) | `maidan-types/src/models.rs` |

## Why

Every serialized `Message` — REST responses, event frames, MCP tool results,
write-acks — carried `"metadata":{}`. The vast majority of messages have no
metadata, so that was pure repeated token cost on the busiest payload in the
system. `Message.content` already skips-when-`None` (Cluster 173); this brings
the same omit-empty treatment to `metadata`, via
`#[serde(skip_serializing_if = "json_value_is_empty", default)]`.

## Why it's safe

- **Serialization-only + idempotent.** `{}` and `null` are both "empty" and both
  omitted; a re-serialize is stable. The store column is unchanged (it still
  holds `{}`), so store round-trips are unaffected — only the *wire* is leaner.
- **`default` on deserialize.** A wire message without `metadata` deserializes
  back to an empty object, so no consumer sees a missing field as an error.
- **Consumers tolerate absence.** The `/ui` metadata readers (`renderSlashResult`,
  `artifactShasFromMetadata`) early-return on a falsy `meta`; the metadata-
  asserting tests only check *non-empty* metadata (which still serializes), and
  the one absence-path test already tolerated it.

## Non-goals

- Full "minimal write-ack" (returning just `{id, posted_at}` instead of the whole
  message) — a semantic change deferred; echoing the (now-leaner) message is the
  low-risk win.

## Exit criteria

- Empty metadata is absent from serialized messages; non-empty is preserved;
  suites green — **met**.
- `v177.0.0` tagged.

## Verification & limits

- `message_serde_tests` (maidan-types unit): `{}` and `null` metadata are both
  omitted and round-trip back to empty; `{"topic":"x"}` is preserved. Full
  `maidan-server` suite (which GETs/lists messages + reads events across many
  e2es) green, confirming no consumer regressed.
- Limit: only `Message.metadata` — other structs' metadata fields (inputs, other
  models) are unchanged.

## References

- [[Retros/Cluster 177.0]]; `maidan-types/src/models.rs`. Program: [[Roadmap]] +
  memory `maidan-next-arc-program` (token round 3).
