# Cluster 256.0 retro — the member's switch

> Tag **`v256.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 20** — Arc I.

## What shipped

- `PUT`/`GET /members/:id/delivery-mode` (`workspace:read` + self-only) over the
  Cluster-254 store: a member switches between `immediate` per-notification emails
  and a periodic `digest`. `SetDeliveryMode` / `DeliveryModeView` DTOs; full new-route
  preflight (OpenAPI + capability-map + matrix). e2e `delivery_mode_rest_e2e`.

## Surprises / decisions

- **Wrapping the enum in the request DTO makes validation disappear.** `SetDeliveryMode
  { mode: EmailDeliveryMode }` — an unknown `mode` string fails serde at the extractor
  and returns `400` before the handler runs, so there's no hand-written parse/reject
  (unlike the email route's `@` check, which the enum has no analogue for). The matrix
  body clause therefore has to supply a *valid* mode (`digest`) so the extractor
  passes and the `cap()` check is what the matrix actually exercises.
- **`GET` is total — no 404.** Email `GET` 404s when unset (an address is genuinely
  absent), but delivery mode always resolves (`immediate` by default), so its `GET`
  just returns the current mode. One fewer state to represent.
- **The enum needs its own schema registration.** Both DTOs reference
  `EmailDeliveryMode`, so it has to be in `components(schemas(...))` too or the
  OpenAPI bijection test fails — the same "register every referenced type" step as
  any body-typed route.

## Capability table extension

| Change | Where |
|--------|-------|
| `PUT`/`GET /members/:id/delivery-mode` + DTOs | `routes/member.rs`, `dto.rs`, `app.rs` |
| OpenAPI + capability-map + matrix | `openapi/*`, `contracts/http-capability-map.json`, `http_capability_matrix_e2e.rs` |

## Risks identified + still open

- None new — a member-scoped self-only route over an existing store method.

## Forward look

**257** the MCP twin (`set_delivery_mode` / `get_delivery_mode` tools) for agent
parity. Then Arc I is essentially complete (optional low-value MCP email tools aside),
and **Program D (scale & durability)** is next.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 255.0]].
