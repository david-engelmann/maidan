# Cluster 256.0 — delivery-mode REST

> **Program C (notifications & reach), part 20** — Arc I. Phase XXIV post-gate
> hardening. Tag **`v256.0.0`**. No new gate tag.

## Goal

Let a member choose their email delivery mode over the API, instead of the
store-only path Cluster 254 left. `PUT`/`GET /members/:id/delivery-mode` — the
member-facing switch between immediate per-notification emails and a periodic
digest.

## Scope

| Change | Where |
|--------|-------|
| `SetDeliveryMode` (request) + `DeliveryModeView` (response) DTOs | `dto.rs` |
| `set_member_delivery_mode` (`PUT`) / `get_member_delivery_mode` (`GET`) handlers, `workspace:read` + self-only | `routes/member.rs` |
| Route registration | `app.rs` |
| Full new-route preflight: OpenAPI path stubs + `paths()`/`components()` regs + `http-capability-map.json` entries + matrix PUT body clause | `openapi/*`, `contracts/*`, `http_capability_matrix_e2e.rs` |

## Design decisions

- **Self-only for a session caller, like the other member-prefs surfaces.** Both
  routes are `workspace:read` + `ensure_acting_member` — a member sets their *own*
  delivery mode; a bearer stays act-as-any (the Cluster-202/203 model, matching the
  notification-prefs and email routes).
- **The request DTO wraps the enum directly, so validation is free.** `SetDeliveryMode
  { mode: EmailDeliveryMode }` deserializes `{"mode":"digest"}` straight to the enum;
  an unknown mode fails deserialization at the extractor → `400`, so the handler
  needs no manual parse-and-reject.
- **`GET` never 404s.** An unset mode reads back as `immediate` (the store default),
  so the read is total — no "unset" state to represent, unlike the email `GET`.
- **`EmailDeliveryMode` registered as an OpenAPI schema.** `DeliveryModeView` and
  `SetDeliveryMode` both reference the enum, so it joins `components(schemas(...))`
  alongside them for the bijection to hold.

## Non-goals / deferred

- **MCP tools for the delivery mode** (Cluster 257) — the parity surface.
- Optional MCP email tools (set/clear the address) — low value, still deferred.

## Risks

- New-route preflight — covered by `openapi_e2e` (bijection) + `http_capability_
  matrix_e2e` (map ↔ router, with the PUT body clause) + the `delivery_mode_rest_e2e`
  behaviour test.
