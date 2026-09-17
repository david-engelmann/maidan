# Maidan — Candidate Roadmap (Advisory)

This is a **research-backed suggestion for sequencing**, not a plan to adopt. Each phase lists candidate initiatives with the dependency logic behind the ordering. Initiative briefs live in `initiatives/`; evidence trails in `research/`; open decisions that need the maintainer live in `DECISIONS.md`.

**Guiding principle for the ordering:** fix what automated agents must trust first (auth correctness, release integrity), then fix what builders touch (SDKs, discovery, quickstart), then polish (docs, voice).

**Repo state (2026-09-17):** `main` @ `ca2ddd3`; tag `v402.0.0` cut; #905–#913 merged. D-TAG, D-1, D-2, D-3 resolved (see `DECISIONS.md`); D-4 and the new D-5 open. `FINDINGS-INDEX.md`'s Status column records per-finding state — check it before scheduling anything.

---

## Phase 0 — Trust inputs (before building on top)

Work here doesn't change features; it determines whether downstream work is safe.

### INIT-02 — Capability-map privilege review (F-06, F-45, P0)
**Fourth-pass rewrite.** The 20 read-gated mutating routes were re-derived mechanically from the map and dispositioned by class: 4 self-scoped by construction (mute routes — no issue); 14 `/members/{id}/…` routes where `ensure_acting_member` is session-only, so a read-scoped Bearer <redacted> can rewrite any member's personal state (Cluster-202 act-as-any — **open decision D-5**); `POST /tokens/attenuate` is holder-side narrowing-only policy with D-2's cascade bounding revocation (not a defect); `POST /workspaces/:wid/dm` creates shared state as an arbitrary member on a read cap (**new F-45** — the sharpest single item, one-line fix class). **Update:** #907/#908 closed P1 #7's hard half (SoD ledger); #909 closed the attenuation parent link. The #904 ratchet remains the established idiom. Evidence: `research/R-05-capability-map-outliers.md`.
**Suggested before:** any roadmap item that assumes ambient agent tokens are safe to hold.

### INIT-07 — Auth enforcement architecture (F-07–F-09, P1/P2)
**Fourth-pass reframe.** The repo already contains better answers than "router-level binding": MCP tools declare `required_capability()` once and enforce + advertise from it; the denial-matrix e2e proves map↔handler agreement both directions; #907/#908 proved the funnel and compile-error patterns. Recommendation rewritten around generalizing those to HTTP. `bypass()` auditability still open. Evidence: `research/R-02-contracts-auth-evidence.md`.

**Phase 0 exit signal:** F-45 fixed; D-5 decided with the maintainer; enforcement generalization has a decision (adopt or deliberately defer).

---

## Phase 1 — Release integrity

### INIT-01 — Versioning and releases (F-01–F-05, P0/P1/P2)
**Update:** tag `v402.0.0` cut (D-TAG resolved); #905's pin values merged; CHANGELOG current through 402; SECURITY.md coherent. What remains is mechanism: the quickstart pins are still four hand-edited values across two files (compose default silently wins over Dockerfile ARG); README's Docker pin is at `:v339.0.0`; `docs/Operations.md` still documents the dead per-cluster tagging scheme; no machine-readable version source. Advisory: generate pins via a `release.yml` follow-up job (pins for vN need vN's tarballs — the follow-up PR is the honest shape), one versions file, `/health` boot-verification in CI, and write down the actual retro→tag cadence. Evidence: `research/R-03-release-forensics.md`, `research/R-06-quickstart-forensics.md`.
**Suggested before:** anything that asks users to "pin to a version" — the version now exists; the pinning machinery should too.

---

## Phase 2 — Builder surface

### INIT-03 — SDK surface and docs (F-10–F-12, P0/P1/P2)
All four SDK READMEs show a claim-response shape (`res.get("thread")`) that main no longer returns — the snippets silently no-op on success. SDKs also lack member/token provisioning, forcing private `_req(...)` fallbacks. Evidence: `research/R-04-sdk-claim-shape-evidence.md`.

### INIT-04 — Scheduler discoverability (F-13, P1)
Task schedules shipped (Clusters 228–229, real sweeper, REST + MCP surface) but appear in no integration doc, example, or SDK README. For agent-automation consumers this is a headline feature hiding in the changelog.

### INIT-11 — CLI trust model (F-42, P1)
**Fourth-pass reframe — the audit's recommendation is withdrawn.** `maidan-cli` is a *server host* (it builds the store and serves MCP over stdio), not an operator console — "rewrite as an HTTP client" is incoherent for this binary. The real sharp edge: no `MAIDAN_MCP_TOKEN` → silent `AuthContext::bypass()` (main.rs:218-224), no warning, no flag — the MCP tools it serves enforce per-tool capabilities only against the ambient context, and the default ambient context is omnipotent. Advisory: explicit bypass opt-in with loud logging, documented trust model. Evidence: `research/R-08-cli-evidence.md`.

**Phase 2 exit signal:** a new builder can copy-paste the SDK claim snippet and get a working lease; scheduler usage is documented where builders look for it; operators can manage a remote instance without database credentials.

---

## Phase 3 — Getting started and ops DX

### INIT-05 — Getting started (F-14–F-20, P1/P2)
Three quickstart variants tell three auth stories; none of the copied commands are CI-verified; the curated `.env.example` is unreferenced from the docs; compose profile/port collisions undocumented; app services lack healthchecks. **Update:** adopt #905's retro as policy — "verified by running, not by reading" (boot the stack, read `/health`); note the compose-default-wins trap (`docker compose config` can't show the drift). Evidence: `research/R-06-quickstart-forensics.md`.

### INIT-08 — Ops and migration hygiene (F-31–F-34, F-43, P1/P2)
`make smoke` is broken (starts only Postgres, then waits forever); sqlite/postgres migration numbering diverges (pg 0097–0099 vs sqlite 0096–0098 on current main); `migrate.rs` hard-codes 197 `include_str!` entries; Helm prod values lack a real image repo/tag. **F-34 now has its incident:** #909's author nearly shipped an unregistered migration against this exact list — the predicted failure happened to the maintainer this week; only a `build.rs`-generated registry removes the class. F-43: the full workspace test run isn't in per-PR CI — #907–#913 stacked six `maidan-store` migrations plus gate changes; whether the full run covered the stack together was not re-confirmed. Consider a merge queue or post-merge full-workspace gate.

**Phase 3 exit signal:** the documented first-run path works verbatim from a clean machine and is covered by CI (including the `/health` version assertion).

---

## Phase 4 — Docs and voice

### INIT-06 — Docs structure and hygiene (F-21–F-30, F-44, P1/P2)
The strategy pack (`Launch.md`, `Adoption.md`, `Handoff.md`) reads as internal memos published as public docs; `Architecture.md`'s API table is a changelog dump; OIDC doc mixes design spike with shipped state. **Updates:** F-22 reframed — `[[wikilinks]]` are a recorded ADR, do not "fix" them; F-23 resolved by the tag (residual: F-44, the `Baseline:` header still reads v349.0.0 — one line); F-29 resolved — `CLAUDE.md`'s "latest `v402.0.0`" is now true. #906 merged, backfilling the Cluster 387 retro — C5 is the measured cost of the one lapsed retro; #913 then followed the retro-before-tag rule exactly.

### INIT-09 — Examples quality (F-35–F-40, P1/P2)
"Runnable" examples that only print tool lists; `a2a_interop.py` assumes an auth-disabled quickstart that hasn't existed since Cluster 313; no example demonstrates the full waiter lifecycle (claim → acknowledge → report_usage → release).

### INIT-10 — Voice and wording (F-41, P2)
Targeted rewrites for ~7 AI-sounding passages, with before/after text. Low risk, good batch work. Evidence: `research/R-07-voice-analysis.md`.

---

## Decision gates (not phases)

Two remain open — they need the maintainer. Tracked in `DECISIONS.md`:

- **D-4** — `set_thread_budget` PUT shape: keep replace semantics or move to PATCH-style.
- **D-5** (new) — does Cluster-202 act-as-any extend to personal-state mutation on least-privilege tokens? (The 14 Class-B routes in INIT-02.)

Resolved and recorded as precedent (read the "Reasoning worth keeping" sections before facing similar trades): **D-TAG** (tag `v402.0.0` cut after retro, per the repo's rule), **D-1** (SoD worker ledger), **D-2** (revocation cascades), **D-3** (tap's two jobs split: cursor for projection, scheduled verifier for verification).

The maintainer's 2026-09-16 framing — *"what's left is the two decisions — the tag, and the three correctness trades"* — is now fully discharged; D-5 is the one new decision the fourth pass surfaced.

---

## Explicit non-goals of this roadmap

- It does not propose API redesigns. The contract discipline (events, capability map, MCP tools) is a strength; the recommendations are about **enforcement, surfacing, and documentation** of the existing design. (D-4 is the deliberate exception: it's recorded as a decision for the maintainer, not a recommendation.)
- It does not propose rewriting the Rust workspace layout (14 crates, `publish = false`, version `0.0.0`). That is a deliberate internal posture; the recommendation is only to make the *external* stability story honest.
- It does not set dates. Sequencing is by dependency, not calendar.
