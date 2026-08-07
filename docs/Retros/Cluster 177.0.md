# Cluster 177.0 retro — omit empty message metadata

> Tag **`v177.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc 4 (token round 3), part 3.

## What shipped

- `Message.metadata` now serializes with
  `#[serde(skip_serializing_if = "json_value_is_empty", default)]` — an empty
  (`{}` or `null`) metadata is omitted from every serialized message (REST,
  events, MCP, write-acks) and deserializes back to an empty object.

## Surprises

- **The blast-radius check was the work, not the change.** The change is one
  serde attribute + a 3-line helper. The care went into confirming it's safe
  everywhere a `Message` crosses the wire: the `/ui` metadata readers already
  early-return on falsy `meta`, the metadata-asserting tests only check
  non-empty content, and the store column is untouched (so round-trips are
  unaffected). Idempotence (`{}` and `null` both omit) means no re-serialize
  drift.

## Decisions

- **Only `Message.metadata`.** It's the highest-volume payload; other structs'
  metadata (mostly inputs) weren't worth the churn.
- **Echo the leaner message, don't switch to a minimal id-only ack.** The
  omit-empty change already trims the write-ack; a semantic minimal-ack is a
  bigger, deferrable change.

## Capability table extension

| Change | Where |
|--------|-------|
| Omit empty `Message.metadata` from serialization | `maidan-types/src/models.rs` |

## Risks identified + still open

- **Low.** Serialization-only, idempotent, `default` on deserialize; consumers
  tolerate absence (verified). Full server suite green.

## Forward look

Arc 4's last item: **opt-in lean event frames** (a subscribe flag that trims the
domain-event frames the WS/MCP streams push). Then the token-round-3 arc — and
the post-v155 four-arc program — is complete.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
