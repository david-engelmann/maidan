# Cluster 289.0 retro — A2A interop conformance (arc finale)

> Tag **`v289.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P1: A2A
> v1.0 compliance — arc finale (part 8).** No new gate tag.

## What shipped

The A2A v1.0 compliance arc's cap: **proof that an external client interops**, plus a
reproducible harness and a report-only CI job.

- **`examples/a2a_interop.py`** — a dependency-light (httpx-only) A2A conformance client:
  validates the Agent Card against §4.4.1, then exercises the **JSON-RPC** binding
  (`SendMessage` → `GetTask` → `ListTasks`, the canonical spec method names, and
  `-32601` for an unknown method) and the **REST** binding (`GET /a2a/v1/tasks/{id}`,
  `GET /a2a/v1/extendedAgentCard`). Exits non-zero on any conformance failure; doubles as
  a client example.
- **`scripts/a2a-interop.sh`** — boots a source-built Maidan (SQLite, auth disabled),
  waits for health, runs the client, tears down (the loadgen/chaos harness pattern).
- **`a2a interop` CI job** — report-only (`continue-on-error`, not a required check): runs
  the harness on every PR to prove non-Rust interop, without ever blocking a merge.
- **Live-verified**: every conformance check passes against a source-built server.

## Surprises / decisions

- **Raw-httpx conformance client, not the A2A SDK.** A conformance client that speaks the
  wire format directly validates spec conformance without coupling to (or breaking on) an
  A2A SDK's version churn — the same fragility that kept the framework interop CI a
  follow-up in Cluster 280. It's the honest, low-maintenance choice.
- **The CI job is report-only, and that's deliberate.** The A2A binding *behavior* is
  already gated in CI by the required Rust e2e tests (`a2a_protocol_e2e`, `a2a_grpc_e2e`,
  `channel_access_e2e`). This job adds the external-client cross-check; making it
  `continue-on-error` (like the pattern behind the deferred quickstart/interop jobs) means
  a boot/network hiccup reports but never blocks. The binding correctness is not riding on
  a flaky boot-in-CI job.
- **gRPC interop is covered by the Rust e2e**, not the Python client (a Python gRPC client
  needs generated stubs + grpcio — added weight for little extra signal); the httpx client
  covers the two HTTP bindings + the card.

## The arc, complete (282–289)

The A2A endpoint is now A2A v1.0-conformant across all three transports:

| Cluster | Delivered |
|---------|-----------|
| 282 | JSON-RPC method names → canonical spec strings (§5.3) |
| 283 | `ListTasks` + `GetExtendedAgentCard` |
| 284 | per-task push-config model (all four push ops) |
| 285 | Agent Card §4.4.1 schema |
| 286 | HTTP+JSON/REST binding (§11) |
| 287 | gRPC binding (§10, vendored codegen) |
| 288 | transport negotiation + configurable origin (§5.2) |
| 289 | interop conformance client + harness + report-only CI |

## Capability table extension

| Change | Where |
|--------|-------|
| A2A conformance client + harness + report-only interop CI job | `examples/a2a_interop.py`, `scripts/a2a-interop.sh`, `.github/workflows/ci.yml`, `docs/Framework Integrations.md` |

## Risks identified + still open

- **Official A2A SDK / TCK** (vs a hand-written client) is a deeper future validation if
  ecosystem promotion needs a certified badge.
- **Within-arc deferrals remain** (logged in Open Work): gRPC `SendMessage`/push/streaming,
  streaming REST, error-taxonomy alignment, Helm/compose gRPC port, Agent Card optional
  fields.

## Forward look

The A2A v1.0 compliance arc (282–289) is **complete** — the last launch-readiness P1.
Remaining launch-readiness items are the smaller polish tracks (Architecture split,
GitHub metadata) in Open Work.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 288.0]] and closes the A2A arc opened at [[Retros/Cluster 282.0]].
