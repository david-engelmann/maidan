# Cluster G — Agent-to-Agent transport

After Cluster F secured HTTP, WebSocket, and MCP, Cluster G connects
**multiple Maidan deployments** so agents on separate instances can share
the same event stream. The first cut is **Maidan-native federation** over
the existing `maidan_events` log — not a full [Google A2A Protocol
v1.0](https://a2a-protocol.org/) binding (deferred; see Out of scope).

> **Goal:** Register remote peers, pull their workspace event tail via
> authenticated HTTP, ingest idempotently into the local bus + event log,
> and expose an inbound push endpoint for low-latency replication. Operators
> manage peers via HTTP.
>
> **Target tag:** `v0.6.0`.

## PRs

| #       | Title                                                                 | Issue |
|---------|-----------------------------------------------------------------------|-------|
| G.1     | `feat(maidan-a2a): federation types, envelope, errors`                | #80   |
| G.2     | `feat(maidan-store): schema 0009 peers + ingest dedupe`               | #81   |
| G.3     | `feat(maidan-server): inbound POST /a2a/v1/events`                    | #82   |
| G.4     | `feat(maidan-a2a): outbound peer client (list_events poll)`           | #83   |
| G.5     | `feat(maidan-server): federation worker + startup wiring`             | #84   |
| G.6     | `feat(maidan-server): peer admin API + federation capabilities`       | #85   |
| G.7     | `feat(maidan-server): well-known agent card for peer discovery`       | #86   |
| G.retro | `docs(retro): Cluster G retrospective + v0.6.0 tag prep`              | #87   |

## Order

1. **G.1** — `maidan-a2a`: `PeerId`, `Peer`, `FederationEnvelope`
   (`origin_peer_id`, `remote_event_id`, `StoredEvent` payload),
   `FederationError`. No I/O yet.
2. **G.2** — migration **0009**: `maidan_peers` (`base_url`, `workspace_id`,
   `token_hash`, `enabled`, `last_synced_event_id`, cursor metadata) +
   `maidan_federated_ingest` (`peer_id`, `remote_event_id` PK) for
   idempotency. Store trait: CRUD peers, record ingest, list enabled peers.
3. **G.3** — `POST /a2a/v1/events` (batch): validate peer bearer (reuse
   `maidan-auth` hash pattern or dedicated peer secret), dedupe, append to
   local `maidan_events`, `bus.publish`. Capability `federation:ingest`.
4. **G.4** — `maidan-a2a::Outbound`: HTTP client calling remote
   `GET /workspaces/:wid/events?after_id=&limit=` with peer token; map to
   envelopes.
5. **G.5** — `FederationWorker` tokio task on server: periodic poll per
   enabled peer, ingest via shared ingest path, update cursor. Config:
   `FEDERATION_POLL_INTERVAL_SECS` (default 30). `FEDERATION_DISABLED=1`
   for tests.
6. **G.6** — `POST/GET/DELETE /workspaces/:wid/peers` admin routes;
   mint peer secret once; capability `federation:admin` (or extend
   `token:admin` — decide in G.6 PR). Peer bearer distinct from member API
   tokens.
7. **G.7** — `GET /.well-known/maidan.json` agent card (name, version,
   `a2a` ingress URL, required capabilities) for operators/scripts.
8. **G.retro** + `v0.6.0` tag.

G.3–G.4 can land together if bisectability cost is high; keep separate
issues for traceability.

## Federation protocol (v0.6.0)

| Direction | Mechanism |
|-----------|-----------|
| Pull     | Outbound poll of remote `list_events` with `after_id` cursor stored on `maidan_peers`. |
| Push     | Optional `POST /a2a/v1/events` with JSON array of `FederationEnvelope`. |
| Auth     | `Authorization: Bearer <peer_secret>` on both directions; SHA-256 stored. |
| Dedupe   | `(peer_id, remote_event_id)` in `maidan_federated_ingest` before append. |

Inbound ingest **re-publishes** to the local bus; it does not replay
remote mutations into entity tables (events are the source of truth for
subscribers). Materialized views / indexer reactions follow from bus, same
as local mutations.

## Capability vocabulary (additions)

| Capability           | Allows                          |
|----------------------|---------------------------------|
| `federation:ingest`  | `POST /a2a/v1/events`           |
| `federation:admin`   | Peer CRUD under a workspace     |

Peer tokens minted in G.6 include `federation:ingest` only; workspace
admin member tokens get `federation:admin` for operator flows.

## Exit criteria

- CI green on `main`.
- Two in-process or docker-compose Maidan instances: peer registered,
  event posted on A appears on B's bus within one poll interval (or
  immediately via push).
- Duplicate delivery of the same `(peer_id, remote_event_id)` is a no-op.
- Invalid peer bearer on inbound returns 401.
- [[Retros/Cluster G]] merged; `v0.6.0` tagged.

## Risks

| Risk                                                                 | Mitigation                                                                 |
|----------------------------------------------------------------------|----------------------------------------------------------------------------|
| Event payload schema drift between versions                          | v0.6.0 same-repo peers only; document version field on agent card.        |
| Infinite ingest loops (A→B→A)                                        | Ingest marks origin; do not re-export events that carry `federated_from`.  |
| Pull load on large backlogs                                          | `limit` cap per poll; cursor advances; standing risk for catch-up tooling. |
| Peer secret sprawl                                                   | Hash-only storage; show secret once at peer create (mirror F.7).           |

## Out of scope (deferred)

- Full **Google A2A Protocol** v1.0 (`SendMessage`, task lifecycle, proto
  bindings) — evaluate post-`v0.6.0` as bridge or parallel ingress.
- Cross-workspace entity creation via federation (only event replication).
- Automatic peer discovery / DNS-SD (static peer registry in v0.6.0).
- Signed event envelopes / non-repudiation (Cluster V).
- Bi-directional CRUD proxy (remote HTTP on behalf of local agents).
