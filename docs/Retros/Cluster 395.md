# Cluster 395 retro — Wave 3 #35: named capability sets + stable `maidan://` URIs

Wave 3 #35 (B20 + B22) asked for **named capability sets**
(`maidan.agent.worker`, `maidan.human.admin`) with progressive grant and
holder-side attenuation (`NEW-cap-attenuation` — Levy/Madden, **not** a
Cedar rewrite), plus **stable `maidan://` room URIs**
(`maidan://{workspace_id}/channels/…/threads/…/messages/…` with an
optional `#sha256:<hex>` fragment), optional `/.well-known/maidan-room`,
and a handle rename that **must not break stored ids**.

Named sets are mint-time recipes. Tokens still hold atomic
`workspace:read` / `message:post` / … strings, so every existing gate
stays valid. Attenuation is the holder path: you can only drop rights
you already hold. `token:admin` remains the issuer and can still mint
any known list.

The room URI authority is **always the workspace UUID**. A handle is an
optional alias on a sidecar table. MCP `maidan://threads/{id}` and
Cluster 392 `maidan:event/{id}` pins stay — `RoomUri` rejects them.

Four impl PRs (395.1–395.4) + this retro. Every PR targets `main`.
**Row #35 is closed.** Do **not** start Wave 3 #36 from this close.

Do **not** cut `v395.0.0` from this PR — the maintainer tags.

## What shipped

- **395.1 (#863) — types.** `RoomUri` hierarchical grammar + optional
  content-hash fragment. `WorkspaceHandle` / `RoomCard`
  (`maidan.room/1`) / `RoomDiscovery` (`maidan.room-discovery/1`).
  Handle syntax `[a-z][a-z0-9-]*`, never a UUID.
- **395.2 (#864) — auth.** Named sets + `expand_set` / `held_sets` /
  `progressive_grant` / `attenuate` / `attenuate_expiry`. Federation
  peer caps stay out of both sets. Amplification fails closed.
- **395.3 (#865) — store.** `maidan_workspace_handles` (pg 0093 /
  sqlite 0092). `set` / `get` / `workspace_id_for_handle`. Unique
  handle → `Conflict`; rename updates the alias only.
- **395.4 (#866) — server + MCP.** Issuer mint accepts
  `capability_set`. `POST /tokens/attenuate` needs `workspace:read`,
  not `token:admin`. Public `GET /.well-known/maidan-room` (scheme
  only). Authenticated room card + handle PUT/GET. `GET /capability-sets`.
  `GET /me` + MCP `whoami` report `capability_sets`. MCP
  `list_capability_sets` / `parse_maidan_uri` / `get_room` /
  `set_workspace_handle` / `attenuate_token`.

## Decisions

- **Recipes, not a second vocabulary.** Expanding a set and storing
  atomics keeps `has_capability` and the capability maps unchanged.
- **Issuer mint stays `token:admin`.** Progressive grant on mint uses
  the full known list as `held` so an admin can grant any named set.
  Levy/Madden is the **holder** path (`/tokens/attenuate`).
- **UUID authority, handle alias.** Putting a handle in the URI host
  would make rename a break — the thing #35 forbids.
- **Well-known is scheme-only.** A tenant list on a public document
  is a directory leak. The room card is authenticated.
- **Sidecar handle table.** Avoids a `row_to_workspace` column ripple.
- **Did not reopen Clusters 392–394.** Room URIs are a new grammar
  beside 392 strong refs. 394 explorer handlers stay as they landed
  (#859–#861); 395.4 (#866) unioned shared catalog / app / contract
  wiring so both surfaces compile together.

## Surprises

- MCP already uses `maidan://threads/{id}` as a resource URI. The
  room grammar must reject a non-UUID host so the two forms stay
  distinct.
- `maidan-auth` has `serde_json` but not `serde`. `CapabilitySet`
  stays in-process; the server DTO serializes.
- `POST /tokens/attenuate` does not collide with `DELETE /tokens/:id`
  (different method). Room OpenAPI stubs live in a new
  `openapi/paths/room.rs` so 394.2's `api.rs` churn stays isolated.
- A digit-leading UUID string fails handle **syntax** before
  `LooksLikeId`. The lookalike test uses a letter-leading UUID.

## Test evidence

- Types: URI round-trip + hash fragment; handle-shaped host rejected;
  MCP / `maidan:event` forms rejected; discovery has no tenant list;
  card URI ignores the handle.
- Auth: sets exclude federation; worker ⊆ admin; `held_sets` needs
  the full expansion; attenuate drops / rejects amplify; progressive
  grant set-then-restrict; expiry cannot outlive the parent.
- Store (SQLite + Postgres): set / rename / unique conflict /
  UUID-lookalike reject; stored workspace id unchanged.
- Server e2e (`room_capability_e2e`): well-known public; mint with
  set + restrict; worker cannot issuer-mint but can attenuate;
  amplify 400; handle rename keeps `maidan://{uuid}`.
- MCP: parse / card / handle / catalog / attenuate + amplify reject.
- Contracts: OpenAPI bijection, HTTP capability map, matrix 403s,
  MCP tool names + capability map. `--no-default-features` compiles.

## Forward look

**Cluster 395 is complete. Row #35 is closed.**

Do **not** start Wave 3 #36 from this retro. #36 is a WASI
slash-handler kind (no-network guest, gas/memory caps) — a different
product. Cluster 394 (Wave 3 #34, #859–#862) is already on `main`;
row #34 stays closed. Do not reopen 392–394 from this close.

Deferred:

- Changing issuer mint so `token:admin` cannot grant rights they do
  not hold (orchestrator model stays).
- A public handle directory (`workspace_id_for_handle` exists; a
  public resolve would leak tenants).
- `/ui` mint chrome for named sets (the `/ui` already claims
  subset-on-mint; server issuer mint was never holder-attenuated).
- Remaining Wave 3/4 rows (#36 WASI slash-handler, Wave 4).

Do **not** cut `v395.0.0` from this PR.

## Acknowledgements

#863 types → #864 auth → #865 store → #866 REST/MCP → this retro.
