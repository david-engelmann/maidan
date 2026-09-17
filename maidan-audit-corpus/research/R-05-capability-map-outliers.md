# R-05 — Capability-map outlier analysis

## Method

Four passes, on four commits:

1. **Audit pass (399.2):** the 285-route capability mapping was scanned for routes whose HTTP method implies state mutation (POST, PUT, PATCH, DELETE) but whose mapped capability is read-level. It reported 22 matches.
2. **Verification pass (399.3, fresh clone):** an independent heuristic re-scan — handlers containing `cap(&auth, WORKSPACE_READ)` cross-referenced against route registrations in `app.rs` — found **16 clear cases**. The heuristic has known blind spots (module-prefixed registrations like `dm::open_dm_conversation`, multi-line route chains). Treat the count as "on the order of twenty," not an exact census.
3. **Third pass (400.5, fresh clone, 2026-09-16 evening):** the three handler-level exemplars re-verified — `token.rs:226` and `dm.rs:95` still `cap(&auth, WORKSPACE_READ)?`; `member.rs`'s `delete_member_email` handler unchanged.
4. **Fourth pass (ca2ddd3, fresh clone, 2026-09-17):** the scan was re-derived **mechanically from `contracts/http-capability-map.json`** instead of heuristically: `surface == "http"`, method not in {GET, HEAD}, capability == `workspace:read`. Result: **exactly 20 routes**. Each was then dispositioned by reading its handler — which changed the finding's shape (see "Class disposition").

## The exact 20 (from the map, 2026-09-17)

`POST /channels/{cid}/mute`, `DELETE /channels/{cid}/mute`, `POST /members/{id}/inbox/read`, `POST /members/{id}/notifications/read-all`, `POST /members/{id}/notifications/{nid}/read`, `POST /members/{id}/notifications/{nid}/snooze`, `PUT /members/{id}/notification-prefs`, `POST /members/{id}/channel-follows`, `DELETE /members/{id}/channel-follows/{cid}`, `POST /members/{id}/thread-follows`, `DELETE /members/{id}/thread-follows/{tid}`, `PUT /members/{id}/email`, `DELETE /members/{id}/email`, `PUT /members/{id}/delivery-mode`, `POST /members/{id}/push-subscriptions`, `DELETE /members/{id}/push-subscriptions/{sub_id}`, `POST /threads/{id}/mute`, `DELETE /threads/{id}/mute`, `POST /workspaces/{wid}/dm`, `POST /tokens/attenuate`.

## Class disposition (handler-verified, 2026-09-17)

**Class A — self-scoped by construction (4).** The mute routes use `auth.member_id` directly (`thread.rs:539` — "Personal to the caller (`auth.member_id`)"; `channel.rs:167`) and take no caller-supplied member id. Safe for every caller type. The audit's original framing was wrong about these.

**Class B — self-scoped for session callers only (14).** The `/members/{id}/…` routes guard `{id}` with `ensure_acting_member` (e.g. member.rs:471), whose contract (`routes/mod.rs:82-88`, Cluster 202) constrains **session callers only**: "A Bearer <redacted> is the orchestrator model and may legitimately act as any member in its workspace (unchanged)." Handler comments say "Self-only" (member.rs:462) — false for token callers. A `workspace:read` token can clear any member's email (member.rs:464-477 verified), rewrite notification prefs, etc.

**Class C — decided policy (1).** `POST /tokens/attenuate` (token.rs:226): holder-side by recorded decision, narrowing-only (no privilege gained), and D-2's cascade (#909) now bounds revocation. Not a defect — reframed.

**Class D — sharp bug (1, new F-45).** `POST /workspaces/{wid}/dm` (dm.rs:89-105): `member_id` and `other_member_id` come from the request body; the handler checks `cap(WORKSPACE_READ)` + `ensure_workspace` and never binds either id to the caller — not even `ensure_acting_member`. It creates shared state (a conversation), unlike Class B's personal prefs.

## Why "re-map to write" is the wrong fix (verified reasoning)

- The denial-matrix e2e (`http_capability_matrix_e2e::every_http_map_route_denies_without_required_capability`) mints a token holding every cap *except* the map's required one — so it catches divergence in **both** directions (handler stricter than map → 403 where success expected; handler weaker → 200 where 403 expected). Map↔handler agreement is proven; the remaining gap is policy, which no test can catch.
- Re-mapping Class A/B to `workspace:write` would break legitimate self-service (a read-scoped agent muting its own notifications) without closing the Class-B token hole, which lives in `ensure_acting_member`'s session-only contract, not in the capability.

## The #904 ratchet (landed, Cluster 400.5) and what followed

The post-Cursor audit's P1 #7 easy half (self-granting governance skills) was closed by #904: widening grants need `channel:admin`, removal deliberately ungated, matching on trimmed/lowercased values, MCP check in the handler (argument-dependent). The hard half (SoD laundering through claim release) was closed overnight by #907/#908 (D-1 resolved — see DECISIONS.md). The ratchet ("tightening keeps the lesser capability, widening needs `channel:admin`") is the established idiom.

## Limits of this analysis

- The scan covered the HTTP surface via the map. The 177 MCP tools carry their own `required_capability` declarations — the same class-disposition pass is still needed there (see INIT-02).
- Class B's severity rests on the Cluster-202 contract reading (token act-as-any is deliberate). Whether it *should* extend to personal-state mutation on least-privilege tokens is D-5, not a finding.
- `PATCH` vs `DELETE /messages/{id}` capability inconsistency (noted in pass 2) was not re-verified in pass 4 — check before citing.
