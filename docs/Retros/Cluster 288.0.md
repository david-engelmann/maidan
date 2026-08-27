# Cluster 288.0 retro — A2A transport negotiation + configurable origin

> Tag **`v288.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P1: A2A
> v1.0 compliance, arc part 7.** No new gate tag.

## What shipped

The Agent Card now advertises Maidan's transports accurately and configurably, which is
the whole of A2A's transport negotiation (§5.2 — a client reads `supportedInterfaces`,
ordered by preference, and picks a transport it supports):

- New `A2aCardConfig` (from the environment at startup, threaded through `AppState`):
  - `MAIDAN_A2A_PUBLIC_ORIGIN` → HTTP interface URLs become **absolute** (e.g.
    `https://maidan.example/a2a/v1/rpc`) instead of host-relative.
  - `MAIDAN_A2A_GRPC_PUBLIC_ADDR` → a **`GRPC` `AgentInterface`** is added to the card
    (`host:port`), so the gRPC binding (Cluster 287) is discoverable.
- `agent_card` (well-known route) and `dispatch_get_extended_agent_card` now read the
  config from `AppState`.
- `docs/Production.md` documents A2A transport deployment (the enable + advertise envs).

## Surprises / decisions

- **Advertised gRPC address is separate from the bind address.** `MAIDAN_A2A_GRPC_ADDR`
  (Cluster 287) is where the server *binds* (often `0.0.0.0:50051`); the card advertises
  `MAIDAN_A2A_GRPC_PUBLIC_ADDR` (the reachable `host:port`). Deriving one from the other
  would be dishonest behind a proxy/LB, so they're distinct, and the gRPC interface is
  only advertised when the public address is explicitly set.
- **Default card is byte-identical to Cluster 285.** With no env set, HTTP URLs stay
  relative and no gRPC interface appears — so every existing Agent Card test passed
  unchanged; the new behavior is purely additive under configuration.
- **Server-side negotiation is just an accurate card.** §5.2 puts transport selection on
  the *client* (read the card, pick a supported binding). The server's job is to
  advertise its real interfaces + versions, which this cluster completes; there's no
  additional server-side negotiation handshake to implement.
- **Helm/compose gRPC templating deferred.** The gRPC server is opt-in; wiring a Helm
  values flag + a service port is deployment convenience, not negotiation, and touches
  the helm-lint/kind-install smoke jobs — documented in Production.md and logged rather
  than templated here.

## Capability table extension

| Change | Where |
|--------|-------|
| Configurable Agent Card transport advertisement: absolute HTTP URLs (`MAIDAN_A2A_PUBLIC_ORIGIN`) + gRPC interface (`MAIDAN_A2A_GRPC_PUBLIC_ADDR`) | `crates/maidan-server/src/a2a_agent.rs`, `crates/maidan-server/src/state.rs`, `crates/maidan-server/src/main.rs`, `docs/Production.md` |

## Risks identified + still open

- **No Helm/compose gRPC templating** — documented; the server is opt-in.
- **gRPC op subset** (from 287) — SendMessage/push/streaming over gRPC still deferred.

## Forward look

Arc finale: **289** — an official A2A SDK/TCK interop CI job (report-only first) that
runs a conformance client against a booted Maidan across the bindings, closing the A2A
v1.0 compliance arc.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 287.0]]. Negotiation model from A2A spec §5.2 (`AgentInterface` /
`supportedInterfaces`).
