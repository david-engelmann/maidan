# Cluster 187.0 — SaaS ops: workspace export / portability

**Theme:** Arc B (multi-tenant SaaS operability), part 3 — a read-side
whole-workspace operation so a tenant can be migrated or archived, not only
deleted.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v187.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `crate::export::build` — assemble the workspace content graph into a JSON bundle | `maidan-server/src/export.rs` (new) |
| `GET /workspaces/:id/export` (gated on `token:admin`) | `routes/workspace.rs`, `app.rs`, OpenAPI + `contracts/http-capability-map.json` |

## Why

The only whole-workspace operations were destructive (purge/erase). There was no
way to get a tenant's data *out* — a portability / migration / archival gap that
matters for "run this for major companies" (and for GDPR data-access requests).

## The fix

`build` reads the collaboration graph and returns flat, id-linked collections
(not deep nesting — easier to diff and re-import): workspace, members, channels
(each with its `channel_members`), threads, messages (paginated per thread to
completeness via `list_messages_after`), message edits, pins, and references
(thread + message sources). DM/group-DM message content is captured for free:
DM threads live in the `__dm__` channel, so `list_threads_for_workspace`
includes them.

- **Excludes secrets** (API tokens, webhook/slash secrets, OIDC/OAuth) and
  operational tables (events, audit, deliveries) — this is user content, not
  credentials or ops state.
- **Auth: `token:admin`.** The bundle spans every channel (private included) and
  every DM, so it's a workspace-admin operation, not a per-member read. Reusing
  `token:admin` (whoever manages a workspace's tokens is its admin) avoids a new
  capability's matrix/contract churn; a dedicated `workspace:export` cap can be
  split out later if the roles diverge.

## Exit criteria

- An admin can `GET` a complete JSON bundle of a workspace's content; a plain
  reader is denied; secrets excluded — **met**.
- `v187.0.0` tagged.

## Verification & limits

- `workspace_export_e2e` (auth enabled): a `workspace:read` token gets `403`; a
  `token:admin` token gets `200` with the workspace, members, channel (+name),
  thread, and message body. `openapi_e2e` bijection stays green (route in the
  utoipa `paths(...)` + `http-capability-map.json`).
- Limits (tracked): **reactions/votes are not exported** (per-message N+1 over a
  large workspace); artifact *blobs* aren't included (metadata via references
  only); the whole bundle is assembled in memory and returned in one response —
  fine for typical workspaces, but a streaming/NDJSON export and a matching
  **import** path are follow-ups.

## References

- [[Retros/Cluster 187.0]]; `maidan-server/src/export.rs`. Program: [[Roadmap]] +
  memory `maidan-next-arc-program` (Arc B).
