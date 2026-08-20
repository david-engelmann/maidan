# Cluster 250.0 — member delivery-email REST

> **Program C (notifications & reach), part 14** — Arc I. Phase XXIV post-gate
> hardening. Tag **`v250.0.0`**. No new gate tag.

## Goal

Let a member register (and clear) their delivery email over HTTP, so email
notifications (Cluster 249) can be opted into without direct store access.

## Scope

| Change | Where |
|--------|-------|
| `PUT`/`GET`/`DELETE /members/:id/email` — set (opt-in) / read / clear (opt-out); `workspace:read`, self-only | `routes/member.rs`, `app.rs`, `dto.rs` |
| Full new-route preflight (OpenAPI + capability-map + matrix PUT body clause) | `openapi/*`, `contracts/http-capability-map.json`, `http_capability_matrix_e2e.rs` |

## Design decisions

- **Presence of an address is the opt-in (Cluster 249), so these are the opt-in
  controls.** `PUT` sets the address (opts in), `DELETE` clears it (opts out), `GET`
  reads it (`404` when unset). Self-only for a session caller (`ensure_acting_member`
  — a member manages their own address), bearer act-as-any — the Cluster-239 model.
- **A light `@` sanity check at the edge.** The store persists as-is and the SMTP
  transport validates fully on send (Cluster 248's decision), but rejecting an
  address with no `@` at `PUT` gives immediate feedback (`400`) instead of a silent
  non-delivery later. Not a full RFC 5322 parse — that stays at the transport.

## Non-goals / deferred

- **MCP** `set/get/delete_member_email` tools — optional surface parity, low value
  (email is human-facing config; an orchestrator provisioning a member's email is an
  edge case). Logged as an optional follow-up; the higher-value Arc I pieces
  (digests, presence-aware routing, `/ui` center) come next.

## Risks

- New-route preflight — the PUT body needs the matrix body clause; covered by
  `openapi_e2e` (bijection) + `http_capability_matrix_e2e`.
