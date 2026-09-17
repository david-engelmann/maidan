# INIT-02 — Capability-map privilege review

**Findings:** F-06 (P0, rewritten fourth pass), F-45 (P0, new fourth pass)
**Research:** `research/R-05-capability-map-outliers.md`
**Repo state (2026-09-17):** `main` @ `ca2ddd3`, tag `v402.0.0` cut. PRs #907–#913 merged overnight: D-1 (SoD ledger), D-2 (revocation cascade), D-3 (tap cursor+verifier), D-TAG all resolved — see `DECISIONS.md`. The fourth pass re-derived F-06 from scratch against `contracts/http-capability-map.json` instead of re-asserting the audit's framing, and the finding changed shape significantly.

## What landed since the third pass

- **#907/#908 (D-1):** the `maidan_thread_workers` ledger and both gates reading it — P1 #7 fully closed. See DECISIONS.md for the reasoning (funnel choke points, compile-error predicates).
- **#909 (D-2):** `parent_token_id` is a real column; `revoke_api_token` revokes transitively. The attenuate endpoint's open half is closed: revocation now reaches children.
- **#904's ratchet** (third pass) remains the established idiom for governance-bearing privilege.

## Problem statement, fourth-pass revision

The outlier scan was re-run mechanically: routes in `contracts/http-capability-map.json` with `surface == "http"`, method not in {GET, HEAD}, capability `workspace:read`. Result: **exactly 20 routes** (the audit's "on the order of twenty" is confirmed). But "20 routes on read" turned out to be four different situations, and only some of them are problems:

**Class A — self-scoped by construction (4 routes, no issue).** `POST/DELETE /threads/{id}/mute`, `POST/DELETE /channels/{cid}/mute`: the handlers use `auth.member_id` directly (`thread.rs:539`, `channel.rs:167`) and take no caller-supplied member id. Safe for every caller type. The audit's original framing was wrong about these.

**Class B — self-scoped for session callers, open for token callers (14 routes, the real F-06).** `/members/{id}/inbox/read`, `/members/{id}/notifications/read-all`, `/members/{id}/notifications/{nid}/read`, `/members/{id}/notifications/{nid}/snooze`, `PUT /members/{id}/notification-prefs`, `POST/DELETE /members/{id}/channel-follows[/{cid}]`, `POST/DELETE /members/{id}/thread-follows[/{tid}]`, `PUT/DELETE /members/{id}/email`, `PUT /members/{id}/delivery-mode`, `POST/DELETE /members/{id}/push-subscriptions[/{sub_id}]`. Each takes `{id}` from the path and guards it with `ensure_acting_member` — whose contract (`routes/mod.rs:82-88`) constrains **session callers only**: "A Bearer <redacted> is the orchestrator model and may legitimately act as any member in its workspace (unchanged)" (Cluster 202). The handlers' doc comments say "Self-only" (e.g. `delete_member_email`, member.rs:462: "Self-only; 404 if none was set") — **true for session callers, false for any Bearer <redacted>**. A `workspace:read`-scoped token can clear any member's email, rewrite their notification prefs, follow/unfollow channels as them.

**Class C — decided policy (1 route, not a defect).** `POST /tokens/attenuate` (token.rs:226): holder-side by recorded decision ("no `token:admin`", Open Work.md), attenuation can only narrow so no privilege is gained, and D-2's cascade now bounds the blast radius (revoking the parent kills the subtree). The audit's original "token-minting on a read cap" framing missed that attenuation is *narrowing-only* — minting a *lesser* token on a read cap is coherent. Reframed, not open.

**Class D — sharp bug (1 route, new F-45).** `POST /workspaces/:wid/dm` (`dm.rs:89`): takes `member_id` **and** `other_member_id` from the request body, checks `cap(WORKSPACE_READ)` + `ensure_workspace`, and never constrains either to the caller. Unlike Class B it doesn't even call `ensure_acting_member`. It creates *shared* state (a conversation), not personal prefs — a read-scoped caller can open DM conversations with an arbitrary member as one side.

## Why the naive "re-map to write" fix is wrong

Re-mapping all 20 to `workspace:write` would break the legitimate self-service pattern (a read-scoped agent muting its *own* notifications is correct behavior) and wouldn't fix Class B anyway — the hole isn't the capability, it's that the self-scoping guard is vacuous for tokens. The capability map and the handlers already agree (the `http_capability_matrix_e2e` denial test proves it in both directions: the deny-token holds every cap *except* the required one, so a handler weaker than the map fails the test). This is a **policy** gap, not a mapping gap — no test can catch it, which is why it needs a decision, not just a patch.

## Advisory recommendation

1. **F-45 first (one line):** `open_dm_conversation` should constrain `body.member_id` to the caller the way `ensure_dm_participant` (dm.rs:38) constrains the read paths — or require `message:post`/`workspace:write` since it creates a conversation. This is the only item in the 20 that creates shared state on a read cap with no caller binding at all.
2. **Decide D-5** (DECISIONS.md): does Cluster-202 act-as-any extend to personal-state mutation on least-privilege tokens? The corpus leans (a) — self-scope token callers to `auth.member_id` on the 14 Class-B routes, matching what the "Self-only" comments already promise — but it narrows the orchestrator model, so it's the maintainer's call. Options (b) capability-gated impersonation and (c) declare-and-document are recorded there.
3. **Add the policy test no test can currently express:** every non-GET route on `workspace:read` must either (i) use `auth.member_id` directly (Class A), (ii) call `ensure_acting_member` *and* be covered by the D-5 decision (Class B), or (iii) carry a justification entry in a checked-in allowlist (attenuate's narrowing-only rationale is the template). A new route that mutates on read without one of the three fails CI. This is enforceable today — the map is machine-readable, and the handler source is greppable.
4. **Do not** re-map Class A or attenuate; do not "fix" the 14 by capability change alone.

## Open questions for the building agent

- D-5 is the whole question for Class B — see DECISIONS.md. Don't implement (a)/(b)/(c) without the maintainer.
- For F-45: is `open_dm_conversation`'s body-supplied `member_id` load-bearing for any orchestrator workflow (a fully-scoped token opening DMs on behalf of members)? If yes, the fix is capability-gating, not self-scoping.
- The MCP surface needs the same class-disposition pass: 177 tools, each with `required_capability` — do any MCP tools mutate on read-level capabilities with caller-supplied member ids? (#904's MCP-side skill check is one data point that per-tool logic exists.)

## Signals of resolution

- F-45 fixed: `open_dm_conversation` binds at least one side to the caller or requires a write-level capability.
- D-5 decided and the 14 Class-B routes match the decision (code + comments agree for token callers, not just session callers).
- The Class-A/B/allowlist policy test exists and runs in CI.
- The denial-matrix test covers any changed routes.
