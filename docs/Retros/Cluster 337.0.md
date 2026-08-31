# Cluster 337.0 retro — REST `GET /me` identity endpoint (audit P1.3)

> Tag **`v337.0.0`**. Phase XXIV (post-gate hardening). **Cluster 6 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The REST twin of Cluster 336's MCP `whoami` tool, closing the self-discovery gap on the HTTP
transport. An agent (or a `/ui` session) handed only a base URL + token can now GET its own
identity — the `member_id` every member-attributed write requires — with no out-of-band
provisioning step.

- **`GET /me`** (`routes/member.rs::get_me`) — returns `{member_id, workspace_id, capabilities,
  is_bearer}` straight from the request's `AuthContext` (no store access). `workspace:read`.
- Full new-route preflight: OpenAPI path stub + `WhoAmI` schema registration + a
  `http-capability-map.json` entry (`GET /me` → `workspace:read`).

## Surprises / decisions

- **Reflects auth, not the store** — same property as the MCP `whoami`: it reveals only the
  token's *own* identity, so no store read, nothing cross-tenant. `is_bearer` = an acts-as-any
  orchestrator bearer vs a pinned session (the distinction that governs acting as other members).
- **No `bypass` field on the REST view.** The MCP `whoami` exposes `bypass` (dev auth-disabled);
  `GET /me` runs behind the auth layer where a reflected `bypass` adds nothing for a real
  deployment, so the DTO stays to the four fields a production caller acts on.
- **Top-level route, no path params** — `/me` needs no matrix path-substitution branch and, as a
  bodyless GET, no `http_capability_matrix_e2e` body clause; the standard cap gate covers it.

## Test evidence

`me_rest_e2e::get_me_reflects_the_callers_identity` (auth ENABLED + a minted bearer: `GET /me`
reflects the exact member/workspace and the token's caps, `is_bearer=true`; the anonymous case
→ 401). `openapi_e2e` (path/schema bijection) + `http_capability_matrix_e2e` (deny-without-cap)
green. fmt + strict clippy + `--all-targets` + bootstrap-strip clean.

## Forward look

Audit P1.3 (agent cold-start) is now complete across both transports. Next: **P1.4** post-path
round-trip reduction (routes/message.rs double-fetches thread+channel; `publish_routed_mentions`
re-resolves the message chain even for zero-mention posts) → **P1.5** egress wire-path tests +
LSN replica CI → **P2** docs/polish (gRPC doc contradiction, tool-count `78→85` drift,
Integration.md flagship-surface gaps).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
