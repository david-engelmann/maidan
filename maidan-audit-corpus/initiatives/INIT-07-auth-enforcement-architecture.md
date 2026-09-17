# INIT-07 — Auth enforcement architecture

**Findings:** F-07 (P1, reframed fourth pass), F-08 (P2), F-09 (P2)
**Research:** `research/R-02-contracts-auth-evidence.md`
**Repo state (2026-09-17):** the fourth pass found that the repo already *contains* the answer to this initiative's original recommendation — in three places. This brief is rewritten accordingly: less "consider router-level binding," more "generalize the patterns that already survived contact with real defects."

## Problem statement, fourth-pass revision

The original finding ("~260 per-handler `cap()` call sites, no router-level binding, fail-open by omission") is factually true and still worth addressing. But the fourth pass found the enforcement story is stronger — and more instructive — than the audit credited:

1. **The MCP side already implements declare-once.** Every MCP tool declares `required_capability()` (`crates/maidan-mcp/src/tools/`), and that single declaration drives *both* `tools/list` filtering (capability-scoped agents see a smaller catalog) *and* enforcement (`auth.require_capability(...)` in the tool). The declaration is the enforcement source. HTTP has the same raw material — `contracts/http-capability-map.json` plus the OpenAPI capability extensions — but the map *documents* while the handlers *enforce*, as two separate writings of the same rule.
2. **The denial-matrix tests prove agreement, both directions.** `http_capability_matrix_e2e::every_http_map_route_denies_without_required_capability` mints a token holding every capability *except* the map's required one. A handler stricter than the map fails (request with the map's cap gets denied); a handler *weaker* than the map also fails (the deny-token still holds the weaker cap, so the request succeeds where 403 was expected). Map↔handler divergence is caught. What no test catches is *policy* error where they agree — which is INIT-02's territory, not this brief's.
3. **#907/#908 proved the two patterns that actually prevent the failure mode.** "A separation-of-duties control that one call site can forget is not a control" → funnel security-relevant writes through **one choke point per backend** (`append_assignment_event`; twelve sites covered, three converted to transactions). "No site can silently keep the old behaviour" → **make forgetting a compile error** (predicates take the fact as a parameter; thirteen call sites break loudly). And #908's post-mortem is the warning label: the first fix patched the *reporting* query while the *enforcing* copy lived in another file — "two copies of a rule, and the one you find by searching is not necessarily the one that runs."
4. **#904 proved per-handler checks are sometimes necessary** (the skill-grant widening depends on the *argument*, so the MCP check lives in the handler, not the static declaration). A router-level binding pure enough to forbid handler checks would have blocked the correct fix.

So the refined problem: HTTP enforcement has ~260 hand-written `cap()` calls whose *agreement with the declared map* is tested but whose *existence* is not structurally guaranteed; the MCP side shows the better shape; and the project's own incident history shows which hardening patterns it actually adopts (choke points, compile errors, single declarations).

## Advisory recommendation (revised)

- **Adopt the MCP shape for HTTP: one declaration, enforced and advertised from it.** The concrete form: route registration (or the existing `http-capability-map.json`) becomes the single source; the OpenAPI capability extensions and the `cap()` calls are generated or mechanically checked against it. The e2e matrix test already proves agreement — the goal is to make *disagreement unrepresentable* rather than *detected*. Note the #904 caveat: the declaration must support argument-dependent checks (a per-route "needs handler-level review" escape with a justification), not pretend they don't exist.
- **Generalize #907's funnel rule:** any new security-relevant write path should funnel through one choke point per backend. Review future PRs against the question "can one call site forget this?"
- **Generalize #908's compile-error rule:** where a security predicate's inputs change, prefer changing the function signature (breaking all call sites loudly) over adding a query inside (silently preserving old behavior at unvisited sites).
- **Keep the `bypass()` question open but sharpen it:** `AuthContext::bypass()` is load-bearing for tests and for the CLI's no-token default (see INIT-11). The ask is auditability — log when bypass is active outside test builds — not removal.
- F-08 (typed `maidan-auth` errors) and F-09 (ws-subscribe-filter schema naming/versioning) stand as written; they are small, uncontroversial, and independent.

## Open questions for the building agent

- What generates what? Options: (a) the JSON map generates the `cap()` calls (codegen — strongest, biggest diff); (b) a CI test asserts every route's handler calls `cap()` with exactly the map's capability (cheaper, allowlist-rot risk — the audit's original interim suggestion); (c) the map is generated *from* the handlers (inverts the authority — probably wrong, since the map is the reviewed policy artifact). The corpus leans (a) for new routes with (b) as the backstop, but the migration cost of (a) across ~260 sites needs sizing.
- How does the declaration express #904-style argument-dependent checks without becoming a second policy language?
- Is `bypass()` auditable today (is there a log line when it's constructed outside tests)?

## Signals of resolution

- A new route without a declared capability fails closed by construction (demonstrated by test), or the (b)-style CI check exists with no allowlist entries older than one release.
- Security-relevant write paths are funneled (review checklist or architecture note cites the #907 rule).
- `bypass()` construction outside test builds is logged.
- F-08/F-09 closed.
