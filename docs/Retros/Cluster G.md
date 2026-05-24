# Cluster G retro — Agent-to-agent federation

> Closing wave for Cluster G · target tag `v0.6.0`.

Cluster F secured single-deployment APIs. Cluster G connects Maidan
instances via Maidan-native federation over the `maidan_events` log:
peer registry, inbound batch ingest, outbound poll, and operator APIs.

## What shipped

- **PR #88** — Cluster G kickoff plan + issues #80–#87
- **PR #89** — `feat(maidan-a2a): federation types, envelope, errors` (G.1)
- **PR #90** — `feat(cluster-g): federation peers, ingest, worker, and admin API` (G.2–G.7)

## What was deferred

| To           | What                                              | Why                                      |
|--------------|---------------------------------------------------|------------------------------------------|
| Cluster H    | Docker-compose two-instance federation smoke      | Exit criteria; manual e2e sufficient v0.6.0 |
| Post-v0.6.0  | Google A2A Protocol v1.0 binding                  | Out of scope per cluster plan.           |
| Post-v0.6.0  | Signed event envelopes / non-repudiation          | Cluster V.                                 |
| Post-v0.6.0  | Persistent peer outbound secrets after restart    | Hash-only storage; in-memory map v0.6.0.   |
| Post-v0.6.0  | Automatic peer discovery (DNS-SD)                 | Static registry in v0.6.0.               |
| Cluster V    | Cross-version payload schema negotiation          | Same-repo peers only for v0.6.0.         |

## Surprises

- **`maidan-a2a::Peer` vs `maidan_types::Peer`** — G.1 validation types live in
  `maidan-a2a`; persisted peers include `token_hash` in `maidan-types`.
- **Outbound poll after restart** — worker needs plaintext bearer; v0.6.0 caches
  secrets in `AppState` at peer create only.
- **Peer bearer on `list_events`** — pull sync requires peers to read the remote
  tail; member middleware extended to accept `PeerContext`.

## Decisions

- **Dedupe key `(peer_id, remote_event_id)`** — `maidan_federated_ingest` before
  re-publish; duplicate batch delivery is a no-op.
- **Separate `/a2a` router** — peer-only middleware on ingress; member auth unchanged.
- **Workspace remap on ingest** — federated `Event` payloads rewritten to the
  local peer's `workspace_id`.
- **Capabilities `federation:ingest` / `federation:admin`** — peer tokens are DB
  rows, not API tokens; admin uses member tokens with `federation:admin`.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Migration 0009 `maidan_peers` + `maidan_federated_ingest` | `v0.6.0`           |
| `maidan-a2a` envelope + batch + `Outbound` client       | `v0.6.0`           |
| `POST /a2a/v1/events` peer ingest                       | `v0.6.0`           |
| `FederationWorker` poll loop                            | `v0.6.0`           |
| `POST/GET/DELETE /workspaces/:wid/peers`                | `v0.6.0`           |
| `GET /.well-known/maidan.json`                          | `v0.6.0`           |
| Peer bearer `GET …/events` for pull sync                | `v0.6.0`           |

## Risks identified + mitigated

- **Duplicate ingest** — `federated_ingest_exists` + `ON CONFLICT DO NOTHING`.
- **Invalid peer on ingress** — 401 without valid bearer; `origin_peer_id` must match peer row.
- **E2E breakage** — `FEDERATION_DISABLED=1` in harnesses; federation e2e separate.

## Risks identified + still open

- **Outbound secret lost on restart** — re-create peer or add env-based secret map.
- **A→B→A loops** — dedupe by remote id only; no `federated_from` marker on events yet.
- **Large backlogs** — fixed `limit` per poll; no catch-up tooling.

## Forward look

Cluster H delivers the web UI, MCP stdio, SSE subscribe, and production polish
(graceful shutdown, request IDs). Cut `v0.6.0` after this retro merges.

## Acknowledgements

Solo cluster. G.2–G.7 shipped in one PR after G.1 types landed.
