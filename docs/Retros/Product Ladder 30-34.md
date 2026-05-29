# Product Ladder 30–34 retro — Hardening & agent transport

> Closing wave for Ladder **30–34** · tags **`v30.0.0`–`v34.0.0`** · **`main`** through PR #206 (`c775be5`).

After the product ladder **17–27** close (`v27.0.0`), clusters **28–29** shipped privacy depth and
message edit. Ladder **30–34** tightened abuse resistance, erasure, deploy, and MCP agent
subscriptions without widening the Slack-shaped core schema.

## What shipped

| Cluster | Tag | PR | Theme |
|---------|-----|-----|--------|
| **30** | `v30.0.0` | #202 | Optional HTTP rate limits (`MAIDAN_RATE_LIMIT_*`), `429` problem+json |
| **31** | `v31.0.0` | #203 | Workspace purge: artifact metadata + blob delete |
| **32** | `v32.0.0` | #204 | `helm/maidan-stack` umbrella (optional Postgres + MinIO) |
| **33** | `v33.0.0` | #205 | MCP resource fan-out on HTTP tombstone + thread FSM |
| **34** | `v34.0.0` | #206 | `Mcp-Session-Id` on `POST /mcp/streamable` |

Cross-cutting: [[Clusters/Product Ladder 30-34]], per-cluster retros **30.0–34.0**,
`CHANGELOG` / `Capabilities` / `Roadmap` / `Remaining Work` refresh.

## What was deferred (intentional)

| To | What | Why |
|----|------|-----|
| [[Clusters/Product Ladder 35+]] | Full MCP streamable bidirectional mux | v34 is correlation header only |
| Ladder 35+ | Workspace row / member / channel delete | FK policy + operator UX |
| Ladder 35+ | MCP fan-out on edit, purge, vote HTTP paths | v33 scoped to tombstone + FSM |
| Ladder 35+ | `helm install` in kind CI | Template smoke sufficient for v32 |
| Ladder 35+ | Per-capability quotas, distributed rate limit | v30 is process-local global bucket |
| Ladder 35+ | UI depth (channel browser, WS tail) | API-first ladder |

## Surprises

- **Rate-limit e2e:** bootstrap `POST /workspaces` counts toward the same client bucket as later GETs — tests need distinct `X-Forwarded-For` keys.
- **Artifact purge:** metadata keyed by `uploaded_by` member, not workspace id — orphans without uploader stay in DB.
- **Helm umbrella:** vendoring `charts/*.tgz` (~136 KiB) keeps CI offline-friendly; `helm dependency update` required locally once.

## Decisions

- **One PR per cluster (30–34)** — reviewable units; five tags in one week is acceptable for a solo maintainer.
- **Ladder doc as index** — [[Clusters/Product Ladder 30-34]] is the checklist; individual Cluster N.0 kickoffs stay minimal.
- **v34 stops before spec-complete streamable HTTP** — session id is the bridge; bidirectional mux is Cluster **35** on [[Clusters/Product Ladder 35+]].

## Capability table extension

| Capability | First available in |
|------------|-------------------|
| HTTP rate limiting (optional) | `v30.0.0` |
| Purge artifact rows + blobs | `v31.0.0` |
| Helm maidan-stack umbrella | `v32.0.0` |
| HTTP tombstone/FSM → MCP resource notify | `v33.0.0` |
| Streamable `Mcp-Session-Id` | `v34.0.0` |

## Risks identified + still open

- Rate limits are not MCP-scoped and not distributed — multi-replica deployments need proxy-level limits or Redis (Ladder 35+).
- Workspace purge is not GDPR-complete until full workspace erasure ships.
- MCP clients expecting full 2024-11-05 streamable transport still need dual `POST /mcp` + notifications until Cluster **35**.

## Forward look

**Active:** [[Clusters/Product Ladder 35+]] — agent-native collaboration OS (transport completion,
DMs, notification router, UI v2, automation, enterprise patterns). Target north star: **`v2.0.0`**
after Ladder **35–58** (see ladder doc).

## Acknowledgements

- Maintainer-driven merges #202–#206; CI green on all five PRs.
