# Cluster 287.0 retro — A2A gRPC binding

> Tag **`v287.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P1: A2A
> v1.0 compliance, arc part 6.** No new gate tag.

## What shipped

The A2A gRPC binding (§10): a **tonic** `A2AService` serving the task read/cancel/list
operations on a **config-gated second port**, as thin adapters over the same
`dispatch_*` handlers the JSON-RPC and REST bindings use.

- **Vendored codegen**: a minimal, self-contained `crates/maidan-server/proto/a2a.proto`
  (service + task messages, no `google.*` imports) is compiled **locally** with
  `tonic-prost-build`; the output is committed at
  `crates/maidan-server/src/a2a_grpc/generated.rs` and `include`d — so **no build-time
  `protoc`** is needed in CI or the image.
- **`GrpcA2a`** implements `GetTask` / `CancelTask` / `ListTasks`: it resolves the caller
  from the gRPC `authorization` metadata (`resolve_bearer`, or bypass when auth is
  disabled), calls the shared op handler, and converts the JSON result to the proto
  `Task` (or the JSON-RPC error code to a `tonic::Status`).
- **Config-gated**: the server only starts when `MAIDAN_A2A_GRPC_ADDR` is set (spawned in
  `main.rs`), so default deployments, CI, tests, and `--no-default-features` builds are
  unaffected.

## Surprises / decisions

- **Risk-first probe changed the risk profile.** Before any code, I added the deps + ran
  `cargo deny`: `tonic`/`prost` were **already in the tree** (via the OTLP exporter) so
  the feared new-dependency risk was gone — but tonic 0.14's **server transport pulls
  axum 0.8** (with `axum-core` + `matchit` 0.8), duplicating maidan-server's axum 0.7.
  Quarantined with one `deny.toml` `skip-tree = { crate = "axum@0.8.9" }` (the
  established pattern), re-evaluated when maidan-server moves to axum 0.8.
- **Vendored, not build-time, codegen.** CI and the Dockerfile have no `protoc`; rather
  than add it to every build path, the proto is compiled locally (protoc is on the dev
  machine) and the generated Rust committed. The minimal proto has **no `google.*`
  imports** (REST annotations omitted, well-known types avoided) so it's self-contained.
- **Scope: task read/cancel/list.** `SendMessage` (metadata `Struct`), push configs,
  streaming (`SendStreamingMessage`/`SubscribeToTask`), the extended card, the Agent Card
  gRPC interface entry, and compose/Helm port wiring are deferred — the last two ride
  with the transport-negotiation cluster (288, which adds the configurable public origin).

## Capability table extension

| Change | Where |
|--------|-------|
| A2A gRPC binding: tonic A2AService (GetTask/CancelTask/ListTasks), vendored codegen, config-gated 2nd port | `crates/maidan-server/src/a2a_grpc/{mod.rs,generated.rs}`, `crates/maidan-server/proto/a2a.proto`, `crates/maidan-server/src/main.rs` |
| tonic/prost/tonic-prost direct deps; deny.toml quarantine for tonic-server's axum 0.8 | `crates/maidan-server/Cargo.toml`, `deny.toml` |

## Risks identified + still open

- **Op subset** — SendMessage, push configs, streaming, extended card over gRPC are
  follow-ups.
- **Second axum (0.8)** accepted via the deny skip-tree — real bloat, isolated to the
  gRPC server; clears when maidan-server upgrades to axum 0.8.
- **No compose/Helm gRPC port yet** — the server is opt-in; wiring rides with 288.
- **Error taxonomy** is Maidan's JSON-RPC codes mapped to `Status` (not the full A2A set).

## Forward look

Arc continues: **288** transport negotiation + configurable public origin (§5.2) — adds
the gRPC (and absolute-URL) `AgentInterface` entries to the Agent Card + compose/Helm
port → **289** official A2A SDK/TCK interop CI.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 286.0]]. Proto shape from the A2A `a2a.proto` service definition; the
deny-quarantine follows the [[maidan-rustsec-h2-aws]] triage pattern.
