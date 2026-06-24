# Cluster 128.0 — A2A delivery robustness

**Theme:** Harden the Agent-to-Agent (A2A) delivery paths, which were
fire-and-forget with no timeout, retry, or error visibility — the top robustness
finding from the v126 hardening scan.

**Ladder:** Post-gate — **Phase XXIV** (hardening), tag **`v128.0.0`**, no new
gate tag. Second cluster of the hardening sweep (127 reconcile → 128 A2A).

**Predecessor:** A2A protocol ingress + persisted tasks (72/79).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **`maidan-a2a` client** | `A2aClient` builder set no timeout → 10s `connect_timeout` (all requests) + 30s per-request timeout on non-streaming `call`. |
| **A2A push (`persist_task`)** | `deliver_a2a_push`: 3× retry + capped exponential backoff, per-attempt logging, `maidan_a2a_push_total{result}` metric. |
| **SSE visibility** | Log the `load_task` failure that silently ended a subscribe stream; log the (near-impossible) SSE-frame serialize failure instead of a silent empty frame. |
| **Tests** | In-file harness: push endpoint fails twice then 200s → assert retry-to-success + give-up-at-max-attempts. |

## Non-goals

- A durable A2A push outbox — the retry is best-effort (bounded, logged,
  counted), not transactional durability. A full outbox is a larger, separate
  effort if push reliability becomes a hard requirement.
- Streaming-request overall timeout — `connect_timeout` covers the hang; the
  stream itself is server-bounded.

## PR ladder (actual)

| # | Title |
|---|--------|
| 128.0.1 | `feat(a2a): delivery robustness — timeouts + push retry/backoff + error visibility` (#347) |
| 128.0.retro | `docs(retro): Cluster 128.0 + v128.0.0 tag prep` |

## Exit criteria

- No A2A request can hang indefinitely (connect timeout) — **met**.
- A2A push failures are retried, logged, and counted — **met** (tests).
- `v128.0.0` tagged after retro.

## Ordering & risks

- **Best-effort, not durable.** The push retry reduces silent drops but is not a
  delivery guarantee; documented as such.
- **Conservative timeouts.** `connect_timeout` (not a whole-request timeout) for
  streaming so a legitimately long stream isn't cut; 30s overall on the
  non-streaming call.

## References

- [[Retros/Cluster 128.0]]; v126 hardening scan
- `crates/maidan-a2a/src/client.rs`, `crates/maidan-server/src/a2a_agent.rs`
