# Cluster 285.0 retro — A2A Agent Card §4.4.1 schema

> Tag **`v285.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P1: A2A
> v1.0 compliance, arc part 4.** No new gate tag.

## What shipped

The Agent Card served at `/.well-known/agent-card.json` (and returned by
`GetExtendedAgentCard`) is now the spec's `AgentCard` object (§4.4.1), not Maidan's
flat placeholder:

- **Before:** `{ name, version, protocolVersion, rpcUrl, ingressUrl, capabilities: [method-names] }`.
- **After (spec §4.4.1):** `name`, `description`, **`supportedInterfaces`** (each an
  `AgentInterface { url, protocolBinding, protocolVersion }` — the JSON-RPC interface
  today, the first entry being preferred), `provider`, `version`, **`capabilities`**
  (an `AgentCapabilities { streaming, pushNotifications, extendedAgentCard }` object,
  all true), `defaultInputModes`, `defaultOutputModes`, and **`skills`** (a described
  `AgentSkill`). New typed structs (`AgentInterface`, `AgentCapabilities`,
  `AgentProvider`, `AgentSkill`) mirror the proto.

## Surprises / decisions

- **`protocolVersion` is per-interface, not card-level.** The proto puts
  `protocol_version` on `AgentInterface`, so the card no longer has a top-level
  `protocolVersion`; it lives at `supportedInterfaces[].protocolVersion` (`"1.0"`).
- **The method list is gone from the card.** The flat `capabilities: [SendMessage, …]`
  array was never spec-shaped; capabilities are now the boolean feature set, and the
  supported operations are implied by the protocol binding. The Cluster-283
  `GetExtendedAgentCard` test assertion (which checked the method array) was updated to
  the structured shape.
- **Interface URLs are host-relative.** The card advertises `"/a2a/v1/rpc"` rather than
  an absolute HTTPS URL because Maidan does not know its own public origin. A deployment
  behind a fixed origin can front these; making the origin configurable is folded into
  the transport-negotiation cluster (288).

## Capability table extension

| Change | Where |
|--------|-------|
| Agent Card is the A2A v1.0 §4.4.1 object (supportedInterfaces + capabilities/skills/provider/modes) | `crates/maidan-server/src/a2a_agent.rs` |

## Risks identified + still open

- **Relative interface URLs** — spec prefers absolute HTTPS in production; needs a
  configurable public origin (cluster 288).
- **Optional fields omitted** — `securitySchemes`/`securityRequirements`, `signatures`
  (JWS), `iconUrl`, `documentationUrl`, per-skill modes. Logged; not required.

## Forward look

Arc continues: **286** HTTP+JSON/REST binding (§11) — new REST routes mapping the same
operations, advertised as a second `AgentInterface` → **287** gRPC binding (§10) →
**288** transport negotiation + configurable public origin (§5.2) → **289** official
A2A SDK/TCK interop CI.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 284.0]]. Shape grounded in the A2A `a2a.proto` `AgentCard` /
`AgentInterface` / `AgentCapabilities` / `AgentSkill` messages.
