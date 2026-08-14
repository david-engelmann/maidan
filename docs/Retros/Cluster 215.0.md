# Cluster 215.0 retro — allowlist-by-default, and a leak the remap already half-fixed

> Tag **`v215.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 14.

## What shipped

- A federation ingest trust policy: `EventKind::federatable()` (allowlist-by-default,
  `ArtifactUpserted` excluded) enforced on ingest, plus a fix for the `MemberJoined`
  nested-workspace-id leak in `remap_event_workspace`.

## Surprises / decisions

- **The remap was already half-doing the right thing.** `remap_event_workspace`
  re-scopes each ingested event to the *local* peer workspace. It remaps the
  top-level `workspace_id` everywhere, and the nested `channel.workspace_id` for
  `ChannelCreated` — but the `MemberJoined` arm passed `member` through untouched,
  so `member.workspace_id` kept the peer's *remote* id. The fix is one line
  (`member.workspace_id = workspace_id`), and the inconsistency with `ChannelCreated`
  right next to it is what made it obvious. The existing federation test didn't
  catch it because its peer's remote and local workspace are the same id (remap is a
  no-op there) — the new unit test uses distinct ids.
- **Allowlist-by-default, enforced by the compiler.** The valuable shape of
  `federatable()` isn't the current classification — it's the exhaustive `match`: a
  new `EventKind` won't compile until someone decides whether a peer may push it.
  That turns "is this safe to federate?" into a mandatory review for every future
  kind, not an opt-in someone might forget.
- **One kind actually fails the allowlist today, and for a real reason.**
  `ArtifactUpserted` is excluded because federation replicates *events*, not artifact
  *blobs* — an ingested `ArtifactUpserted` announces a `sha256` whose bytes never
  arrive. Accepting it would create a dangling reference (and a cross-peer existence
  oracle for content-addressed blobs). So the guard rejects something concrete, not
  just hypothetical future kinds.
- **Both ingest paths share one chokepoint.** The push endpoint (`POST /a2a/v1/events`)
  and the pull worker (`poll_once`, which fetches a peer's events and ingests them)
  both route through `ingest_envelope`, so the allowlist check lives in exactly one
  place and covers both.

## Capability table extension

| Change | Where |
|--------|-------|
| Federation ingest trust policy: `EventKind::federatable()` allowlist + `MemberJoined` nested-workspace remap fix | `maidan-types/src/events.rs`, `federation.rs` |

## Risks identified + still open

- **Egress is unchanged** (by design) — the trust boundary is ingest; each side
  allowlists what it accepts, so a peer running this allowlist rejects our
  `ArtifactUpserted` on its own ingest. Filtering egress would save bandwidth but
  isn't a trust requirement (logged, not done).

## Forward look

Program A's last item is the **RLS spike** — evaluate Postgres row-level security as
defense-in-depth beneath the app-layer RBAC (the 160–165 arc). Then Programs B
(agentic orchestration), C (notifications & reach), D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows the completed
transactional-outbox refactor ([[Retros/Cluster 214.0]]).
