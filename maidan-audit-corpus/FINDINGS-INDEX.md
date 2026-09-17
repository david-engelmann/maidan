# Findings Index

Every finding from the 2026-09-16 audit of `main` @ `e232c73`. Severity scale is defined in `README.md`. The "Initiative" column points to the brief that carries the full context, advisory recommendation, and open questions. The **Status** column records the repo state as of the fourth pass (2026-09-17, `main` @ `ca2ddd3`, tag `v402.0.0` cut, PRs #905–#913 merged).

**Verification notes:**
- *Second pass (2026-09-16):* the highest-stakes claims — F-06's three exemplar routes, F-10's claim-shape mismatch, F-17, F-19, F-31, F-32, F-34, and the new F-42 — were re-verified against a fresh clone at `207047c` (Cluster 399.3). Discrepancies found in that pass are corrected below; corrected rows are marked ▼.
- *Third pass (2026-09-16 evening):* re-verified against `dab377a` (Cluster 400.5) and the bodies of PRs #905 (quickstart pin) and #906 (Cluster 387 retro). Status changes and reframes from this pass are marked ◆. Three findings were **reframed** (F-22, F-23, F-29) where the audit had misread a deliberate decision as a defect; one finding was **added** (F-43).
- *Fourth pass (2026-09-17):* re-verified against a fresh clone at `ca2ddd3` with tag `v402.0.0` confirmed via `git ls-remote`. Nine PRs merged overnight (#905–#913): D-TAG, D-1, D-2, D-3 all resolved (see `DECISIONS.md`). F-06 was re-derived mechanically from `contracts/http-capability-map.json` (exactly 20 routes) and **rewritten as a four-class disposition** — the audit's framing was imprecise; two findings were **added** (F-44, F-45); two were **reframed** (F-07, F-42) where the deep dive found the repo already contains better patterns than the audit recommended. Rows changed in this pass are marked ◇.

Status values: `open` · `landed` (PR/commit given) · `reframed` (the finding as originally stated was wrong; the corrected reading is in the initiative brief) · `resolved` (fixed by repo action, nothing left to do).

## P0 — Blocks safe adoption / silently wrong

| ID | Finding | Location | Initiative | Status |
|----|---------|----------|------------|--------|
| F-01 ◇ | Release tagging stalled: latest tag v349.0.0 vs `main` 261 commits ahead; CHANGELOG at 398.0.0; SECURITY.md support window un-actionable; `GET /threads/:id/deliveries` (Cluster 379) in *no* published release | tags, `CHANGELOG.md`, `SECURITY.md` | INIT-01 | landed — tag `v402.0.0` cut at `ca2ddd3` (2026-09-17, D-TAG resolved); CHANGELOG has 400/401/402 entries; SECURITY.md "latest tag" promise coherent again. Residuals: F-03/F-04/F-05 |
| F-06 ▼◆◇ | **Rewritten fourth pass.** Exactly 20 non-GET routes on `workspace:read` (mechanically derived from the map): 4 self-scoped by construction (mute routes, `auth.member_id` — no issue); 14 `/members/{id}/…` routes where `ensure_acting_member` is session-only so a read-scoped Bearer <redacted> can rewrite any member's personal state (Cluster-202 act-as-any — open decision D-5); `POST /tokens/attenuate` is holder-side narrowing-only policy (D-2 resolved — not a defect); `POST /workspaces/:wid/dm` creates shared state as an arbitrary member on read (new F-45) | `contracts/http-capability-map.json`; `routes/member.rs`, `src/dm.rs:89` | INIT-02 | open — F-45 unfixed; D-5 undecided |
| F-10 | All four SDK READMEs show wrong claim response shape (`res.get("thread")`); snippets silently no-op on success | `sdk/python/README.md`, `sdk/typescript/README.md`, `sdk/go/README.md`, `sdk/rust/README.md` | INIT-03 | open |
| F-45 ◇ | **New.** `POST /workspaces/:wid/dm` takes `member_id` + `other_member_id` from the request body, checks only `cap(WORKSPACE_READ)` + `ensure_workspace` — never binds either id to the caller (not even `ensure_acting_member`). Creates *shared* state (a conversation) as an arbitrary member on a read capability. One-line fix class (constrain `member_id` to the caller or require write-level cap); check whether orchestrator workflows depend on the current shape first | `crates/maidan-server/src/dm.rs:89-105` | INIT-02 | open |

## P1 — Needs rework

| ID | Finding | Location | Initiative | Status |
|----|---------|----------|------------|--------|
| F-02 ◇ | Quickstart binary pinning (v312.0.0 + hand-maintained SHAs) guarantees staleness — and v312 predates the entire Cluster-397 authorization remediation | `compose.quickstart.yaml`, `docker/Dockerfile.quickstart` | INIT-01 | landed (values) — #905 merged: pin → v349.0.0 in both files + both SHAs, verified by running. **Mechanism still open:** four values hand-edited across two files; compose default silently wins over Dockerfile ARG |
| F-03 | README Docker image pinned to `:v339.0.0` while the release is v402.0.0 | `README.md:219` | INIT-01 | open |
| F-07 ▼◇ | **Reframed.** ~260 per-handler `cap(&auth, …)` call sites is true, but the deep dive found the repo already holds better answers than the audit's "router-level binding": MCP tools declare `required_capability()` once and enforce + advertise from it; the denial-matrix e2e proves map↔handler agreement both directions; #907/#908 established the funnel and compile-error patterns. Recommendation rewritten around generalizing those. `bypass()` auditability still open | `crates/maidan-server/src/routes/`, `crates/maidan-mcp/src/tools/` | INIT-07 | open — patterns identified, not yet generalized to HTTP |
| F-11 | No SDK surface for member provisioning / token minting; `lease_demo.py` falls back to private `_req(...)` | `sdk/*/src/*`, `examples/lease_demo/lease_demo.py` | INIT-03 | open |
| F-13 | Task schedules (REST + MCP, real sweeper) undiscoverable: absent from Integration.md, examples, SDK READMEs | `docs/Integration.md`, `examples/`, `sdk/*/README.md` | INIT-04 | open |
| F-14 | README quickstart: three variants, three auth stories, copied commands not CI-verified; `MAIDAN_BOOTSTRAP=1` confusion; one-liner can't mint tokens | `README.md` | INIT-05 | open |
| F-21 | Strategy pack (`Launch.md`, `Adoption.md`, `Handoff.md`) reads as internal memos published as public docs | `docs/Launch.md`, `docs/Adoption.md`, `docs/Handoff.md` | INIT-06 | open |
| F-32 | `make smoke` broken: compose-up without profile starts only Postgres, then waits on a server that never starts | `Makefile`, `compose.yaml` | INIT-08 | open |
| F-35 | LangChain/AutoGen examples labeled "Runnable" but only print tool lists; `mcp<2` pin aging and unenforced | `examples/langchain_maidan.py`, `examples/autogen_maidan.py` | INIT-09 | open |
| F-36 | `a2a_interop.py` assumes auth-disabled quickstart, stale since Cluster 313 made quickstart auth-on | `examples/a2a_interop.py` | INIT-09 | open |
| F-42 ▼◇ | **Reframed — the audit's recommendation is withdrawn.** `maidan-cli` is a *server host* (builds the store, serves MCP over stdio), not an operator console — "rewrite as an HTTP client" is incoherent for this binary. The real sharp edge: no `MAIDAN_MCP_TOKEN` → silent `AuthContext::bypass()` (main.rs:218-224), no warning logged, no flag required — every MCP tool it serves enforces per-tool capabilities only against the ambient context, and the default ambient context is omnipotent | `crates/maidan-cli/src/main.rs` | INIT-11 | open — make bypass explicit (flag + loud log), document the trust model |

## P2 — Nits (batch-worthy)

| ID | Finding | Location | Initiative | Status |
|----|---------|----------|------------|--------|
| F-04 | `docs/Operations.md` documents obsolete v0.X.0-per-cluster tagging ("Cluster close") | `docs/Operations.md` | INIT-01 | open |
| F-05 | No single machine-readable "latest releasable version" source of truth | repo root / release workflow | INIT-01 | open |
| F-08 | Capability validation errors are `String`-typed in `maidan-auth` | `crates/maidan-auth/src/*` | INIT-07 | open |
| F-09 ▼ | `ws-subscribe-filter` schema doesn't name the gating `event:subscribe` capability; `$id` v3 untied to `event-kinds.json` | `contracts/ws-subscribe-filter.schema.json` | INIT-07 | open |
| F-12 | `sdk/python/tests/test_client.py` only asserts `None`-or-`dict` for claim responses | `sdk/python/tests/test_client.py` | INIT-03 | open |
| F-15 | No-Docker quickstart never states the Rust toolchain prerequisite; insecure variant's two-flag requirement buried in collapsed `<details>` | `README.md` | INIT-05 | open |
| F-16 | "One command (Docker)" is actually three steps with manual token copy/paste | `README.md` | INIT-05 | open |
| F-17 ▼ | `.env.example` exists and is curated, but is unreferenced from the README quickstart and `docs/Production.md` — new operators never find it | `.env.example`, `README.md`, `docs/Production.md` | INIT-05 | open |
| F-18 | Compose profile/port collisions (`maidan-a` vs `maidan-server` on 8080) undocumented; no ports matrix | `compose.yaml` | INIT-05 | open |
| F-19 ▼ | App services lack `healthcheck:` blocks (infra services have them); `compose.quickstart.yaml` has none | `compose.yaml`, `compose.quickstart.yaml` | INIT-05 | open |
| F-20 | Apple Silicon support undocumented outside `docs/Pi.md` | `docs/Pi.md` vs `README.md` | INIT-05 | open |
| F-22 ◆ | **Reframed — not a defect.** Obsidian `[[wikilinks]]` are a recorded decision (`docs/Decisions.md`: "Docs vault lives in `docs/` and uses Obsidian wikilinks"; graceful degradation on GitHub accepted; revisit at Cluster H). The original recommendation (rewrite as relative links) is withdrawn. | `docs/Decisions.md` | INIT-06 | reframed — no action unless the Cluster H docs-generator decision changes the calculus |
| F-23 ◆◇ | **Resolved.** The "v349.0.0" baseline lag was the tag gap (D-TAG), not doc negligence — and the tag gap is closed: #913's "Known state" reads "released as `v402.0.0`." Residual → F-44 | `docs/Open Work.md` | INIT-06 | resolved |
| F-24 | `docs/Architecture.md` API surface table is a changelog dump, not an architecture reference | `docs/Architecture.md` | INIT-06 | open |
| F-25 | `docs/OIDC.md` mixes design spike with shipped state | `docs/OIDC.md` | INIT-06 | open |
| F-26 | `docs/Production.md`: duplicate `MAIDAN_SESSION_SECRET` rows; `/metrics` row misplaced inside env-var table | `docs/Production.md` | INIT-06 | open |
| F-27 | `docs/Operations.md` coverage-floor history stale vs CI's 40% floor; lacks maintainer-audience banner | `docs/Operations.md` | INIT-06 | open |
| F-28 | `docs/Threat-Model.md` stale "for Maidan v1.1.0" header line | `docs/Threat-Model.md` | INIT-06 | open |
| F-29 ◆◇ | **Resolved.** `CLAUDE.md:29` now reads "latest `v402.0.0`" — and the tag exists (confirmed via `git ls-remote`, 2026-09-17). The pointer is true. Lesson preserved (never tag syntax for a non-tag); nothing left to fix | `CLAUDE.md` | INIT-06 | resolved |
| F-30 | `k8s/README.md` vs `docs/Deploy.md` disagree on production path (`overlays/prod`) | `k8s/README.md`, `docs/Deploy.md` | INIT-06 | open |
| F-31 ▼ | `helm/maidan/values-prod.yaml`: `pullPolicy: Always` with an empty `image:` block (no repository/tag) | `helm/maidan/values-prod.yaml` | INIT-08 | open |
| F-33 | SQLite/Postgres migration numbering diverges after 0014 | `crates/maidan-store/migrations/*` | INIT-08 | open |
| F-34 ◇ | `migrate.rs` hard-codes 197 `include_str!` entries (was 191; +6 from Clusters 401–402) | `crates/maidan-store/src/migrate.rs` | INIT-08 | open — **strengthened:** #909's author nearly shipped an unregistered migration against this exact list (script asserted on a `cargo fmt`-collapsed const form; "an unregistered one silently does nothing"). The predicted failure happened to the maintainer this week |
| F-44 ◇ | **New (P2).** #913 updated Open Work.md's "Known state" to `v402.0.0` but missed the `Baseline:` header line (:7), which still reads `v349.0.0` | `docs/Open Work.md:7` | INIT-06 | open — one-line fix |
| F-37 | `lease_demo` leaves leases dangling; no example runs the full waiter lifecycle (claim → acknowledge → report_usage → release) | `examples/lease_demo/` | INIT-09 | open |
| F-38 | Examples say "~78 tools" / "well over a hundred" vs actual 177 MCP tools | `examples/*`, `examples/Framework Integrations.md` | INIT-09 | open |
| F-39 | Hero filter zero-match prints "0-tool hero loop" with no warning | `examples/hero_tool_filter.py` | INIT-09 | open |
| F-40 | `lease_demo` `CLAIM=` parsing assumes last stdout line; `rest_maidan.py` lacks seed commands; `examples/README.md` vague "see the docs page" link | `examples/lease_demo/*`, `examples/rest_maidan.py`, `examples/README.md` | INIT-09 | open |
| F-41 | ~7 AI-sounding passages identified with before/after rewrites | `README.md`, `docs/Integration.md`, `CLAUDE.md`, `docs/Architecture.md` | INIT-10 | open |
| F-43 ◆ | **New.** The full workspace test run is not part of per-PR CI. #900–#903 all touch `maidan-store`; each was green only against its own base; the full run was still in flight at handoff. Maintainer-flagged, 2026-09-16. | CI workflow / merge process | INIT-08 | open |
