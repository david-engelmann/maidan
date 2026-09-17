# Decisions

This file tracks the items the audit corpus will **not** resolve unilaterally — trades between good properties, or calls only the maintainer can make. **Fourth pass (2026-09-17):** four of the five original decisions have been *taken* by the maintainer overnight (Clusters 401–402, PRs #907–#913, tag `v402.0.0` cut). They are recorded below as **resolved**, with the reasoning preserved — the reasoning is the durable value: it shows how this project actually decides, which is what the building agent should internalize. Two items remain open: D-4 (carried over) and D-5 (new, from the fourth-pass deep dive).

**Source for resolutions:** commit messages of #907–#913 on `main` @ `ca2ddd3`, read 2026-09-17; `git ls-remote` confirming tag `v402.0.0` at `ca2ddd3`.

The corpus's standing rule still applies: the building agent's active roadmap takes precedence, and open items are recorded as **questions**, not verdicts.

---

## D-TAG — ✅ RESOLVED: tag `v402.0.0` cut

**What happened.** PR #913 ("docs(retro): close clusters 400, 401 and 402 ahead of the tag") wrote the three missing retros, moved `CLAUDE.md` and `docs/Open Work.md` "latest" pointers to `v402.0.0`, and the maintainer then cut tag **`v402.0.0`** at `ca2ddd3` (confirmed via `git ls-remote`, 2026-09-17). The repo's own rule — "Retro is mandatory; release tag never cut without it" (`docs/Decisions.md`) — was followed exactly: retro first, tag second.

**Reasoning worth keeping.** The retro-before-tag order isn't ceremony: #913 notes "writing them is also writing the release notes, which is the point of doing it in this order rather than after." The tag answered the cadence question implicitly (batched: one tag covering clusters 400–402), but the *forward* cadence (per-cluster? batched? on-demand?) is still undocumented — `docs/Operations.md`'s v0.X.0-per-cluster scheme (F-04) remains the only written statement and is now doubly stale.

**Residuals for INIT-01:** F-03 (README `:v339.0.0` pin), F-04 (Operations.md scheme), F-05 (no machine-readable version source), and the quickstart pin mechanism (still hand-maintained; #905 fixed values only). `SECURITY.md`'s "latest tagged release" support promise is coherent again now that a tag exists.

**Related findings:** F-01 (landed), F-03, F-04, F-05, F-23 (resolved), F-29 (resolved).

---

## D-1 — ✅ RESOLVED: the SoD worker ledger (P1 #7, hard half)

**What happened.** #907 (`085ed7c`, Cluster 401.1) added `maidan_thread_workers` (pg 0097 / sqlite 0096): append-only, answers "has this member ever held this thread?", never cleared by release or unassign, removed only with the thread. #908 (`78ac823`, Cluster 401.2) made both gates read it — review report, review enforcement, land-gate standing, land-gate enforcement, on both backends — closing P1 #7.

**Reasoning worth keeping** (from the commit messages — this is the project's decision style at its best):
- *Rejected: the event log.* It already records assignment changes and would need no new table — but Cluster-186 retention prunes it, and "a security control cannot depend on evidence that ages out."
- *Rejected: a `last_worked_by` column.* Smaller but wrong twice over: it remembers only the most recent holder, and "a Bearer <redacted> is act-as-any by design (Cluster 202), so an agent could claim as another member, overwrite the column, and approve. A ledger accumulates and nothing can un-write it."
- *Where the write lives is the load-bearing part.* Every event-emitting assignment path funnels through `append_assignment_event` — one write site per backend instead of three — because "a separation-of-duties control that one call site can forget is not a control, and forgetting a site is the exact shape of every defect the 397.x audit found." The three non-event `Store`-trait variants (`assign`/`claim`/`claim_next`) were converted to transactions recording there too. Twelve sites, all covered.
- *Make forgetting a compile error.* #908's predicates (`is_qualifying_pass` / `standing_land` / `land_gate_standing`) take the worker fact as a parameter rather than querying — "adding it is a compile error at all thirteen call sites, which is the point: no site can silently keep the old behaviour."
- *The test caught the first version doing nothing.* The initial patch fixed the review-status *reporting* query while the FSM close gate kept its own *enforcing* copy in `thread_transitions.rs` — "two copies of a rule, and the one you find by searching is not necessarily the one that runs" (same duplication hazard as Cluster 181's private `parse_kind` copy). The land gate reads the ledger on the transition's own transaction, so a release committing between check and close cannot slip through.

**Related findings:** F-06, F-07. **Related initiatives:** INIT-02, INIT-07.

---

## D-2 — ✅ RESOLVED: revocation cascades to derived tokens (P1 #8, open half)

**What happened.** #909 (`500e641`, Cluster 401.3) made `parent_token_id` a real column (pg 0098 / sqlite 0097), recorded at attenuation time, and `revoke_api_token` now revokes the subtree transitively. ADR recorded: *"Revoking a token revokes everything derived from it."*

**Reasoning worth keeping:**
- *Cascade at revoke time, not auth time.* "One traversal on revoke, versus a recursive query in the hot auth path on every request. Attenuation requires a *live* parent, so a child cannot appear after its parent is revoked; there is no window for write-time cascade to miss."
- *Stopping one level down is the same leak one generation later* — hence transitive, not single-level.
- *No field on `NewApiToken`.* "It is built at 109 sites, 100 of them tests, and only attenuation has a parent — a field would be a hundred mechanical edits serving one caller, the ripple Cluster 173 hit on `NewMessage`. `create_attenuated_api_token` has zero blast radius."
- *`ON DELETE SET NULL`, never CASCADE* on the parent FK: deleting a parent row severs the link, it does not delete children. Already-revoked rows are skipped so "when was this killed" stays true.
- *Near-miss with a lesson for INIT-08:* the migration was almost never registered — the author's script asserted on a two-line `const` form that `cargo fmt` had since collapsed to one line, and "migrations are a hardcoded `include_str!` list, so an unregistered one silently does nothing." (This is F-34 happening to the maintainer in real time; see INIT-08.) The author's process fix: "I now read the file back after scripted edits instead of trusting the script's own success message."

**Related findings:** F-06 (attenuate endpoint — now decided policy: holder-side, cascade-bounded), F-34. **Related initiative:** INIT-02.

---

## D-3 — ✅ RESOLVED: the tap's two jobs were split (cursor + scheduled verifier)

**What happened.** #910 (Cluster 402.1) keyed tap faults by workspace — one tenant's broken chain faults that tenant (`Ok(false)`, row never projected, walk continues) instead of failing the whole indexer; the high-water still advances past a faulted tenant's rows. #911 (Cluster 402.2) gave the tap a persisted cursor (pg 0099 / sqlite 0098): projection walks forward from it; seeded with 0 it is the old full walk, which is what a rebuild wants. #912 (Cluster 402.3) added the opt-in scheduled chain verifier (`MAIDAN_CHAIN_VERIFY_SECS`; unset = nothing runs).

**Reasoning worth keeping** — the resolution is better than the decision's framing:
- *"The framing 'cursor vs re-walk' treats the tap as doing one job. It does two. **Projection** (keep the index current) wants to be incremental; **verification** (notice a tampered log) wants a full walk. Projection was paying verification's cost and inheriting its failure mode — stop everything."*
- The cursor does not weaken per-event checking: every projected event is still chain-checked against its workspace's previous link, and on resume that predecessor is **re-derived from the log, not stored beside the cursor** — "because a second copy of the hashes could drift from the log it is meant to attest." (Without that seeding, a mid-chain resume would skip the `prev_hash` comparison entirely.)
- #912's honesty clause: "Cluster 402.2 moved whole-chain verification out of the search tap's restart path, arguing that verification-by-restart is an accident rather than a control — you cannot schedule it, alert on it, or say when it last ran. **That argument only holds if something does schedule it.** Without this, I would have removed a weak control and pointed at an endpoint nobody runs." Hence the sweeper — but **no default interval**: "a verifier that silently started itself would add a periodic full-log read to every deployment that merely upgraded." And `broken` vs `error` are separate outcomes: "an alert that cannot tell them apart gets ignored, and collapsing them would let a DB blip read as corruption."

**Related initiative:** none in this corpus (surfaced here so the reasoning isn't lost); it belongs wherever the search/indexer roadmap lives.

---

## D-4 — OPEN: `set_thread_budget` is PUT-shaped; raising one cap clears three (API shape)

**Context** (`docs/Open Work.md`, still listed as a decision on `main` @ `ca2ddd3`). The replace semantics are deliberate and documented — the catalog says "omitted dimensions are unbounded," and the store does a full `ON CONFLICT DO UPDATE SET` of every `max_*` column. That is a defensible PUT-shaped API and is not the bug. The sharp edge: because omission is load-bearing ("remove this limit"), raising one dimension requires restating the rest — `set_thread_budget{thread_id, max_tokens: N}`, intending to raise one cap, clears the other three. Documented, so a sharp edge rather than a defect.

**The trade.** Whether a safety envelope should be PUT-shaped at all: replace semantics are simple to reason about but turn partial updates into foot-guns on a safety control; PATCH/merge semantics are forgiving but make "remove this limit" inexpressible without a sentinel. (The adjacent unknown-fields hazard — a typo'd field silently disarming a cap — was closed uniformly in Cluster 398.6/#890 via `deny_unknown_fields`; the shape question is what's left.)

**Related initiative:** none in this corpus; recorded here because it came from the same decision table.

---

## D-5 — OPEN (new, fourth pass): does Cluster-202 act-as-any extend to personal-state mutation on least-privilege tokens?

**Context.** The fourth-pass deep dive dispositioned F-06's 20 read-gated mutating routes by class (see INIT-02): 4 mute routes use `auth.member_id` directly (self-scoped for every caller type — no issue); `POST /tokens/attenuate` is holder-side by decided policy (D-2 resolved — not a defect); `POST /workspaces/:wid/dm` creates shared state as an arbitrary member on a read cap (new F-45). The remaining **14 routes** (`/members/{id}/inbox/read`, `/notifications/*`, `/notification-prefs`, `/channel-follows/*`, `/thread-follows/*`, `/email`, `/delivery-mode`, `/push-subscriptions/*`) take a member id from the path and guard it with `ensure_acting_member` — whose doc comment (`routes/mod.rs:82-88`) states the guard constrains **session callers only**: "A **Bearer <redacted>** is the orchestrator model and may legitimately act as any member in its workspace (unchanged)" (Cluster 202). The handlers' own doc comments say "Self-only" — true for session callers, **false for any Bearer <redacted>**.

**The trade.** A `workspace:read`-scoped token can today rewrite any member's personal state (clear their email, rewrite notification prefs, follow/unfollow channels as them). Three readings:
- (a) **Self-scope token callers too**: on these 14 routes, a token acting as a different member than `auth.member_id` is rejected (or coerced). The handlers' "Self-only" comments already promise this policy; the diff is small; the cost is narrowing the orchestrator model for least-privilege tokens.
- (b) **Gate cross-member action on a capability**: e.g. a new `member:impersonate`, or reuse of an existing admin capability. Preserves act-as-any for properly-scoped orchestrator tokens; bigger design surface (new capability, map entries, matrix tests).
- (c) **Declare act-as-any absolute** and fix the 14 "Self-only" comments to say what the code does. Honest, zero-code — but it ratifies read-scoped tokens mutating other members' state, which least-privilege consumers will not expect.

**The corpus's advisory lean** is (a): the comments already state the policy, the check already exists for session callers, and least-privilege tokens are the audience this corpus cares about. But it changes the Cluster-202 contract, so it needs the maintainer.

**Related findings:** F-06 (rewritten), F-45. **Related initiative:** INIT-02.

---

## How to use this file

If you are the building agent: the resolved decisions are **precedent** — when you face a structurally similar trade, the "Reasoning worth keeping" sections show the shape of answer this project accepts (rejected alternatives with reasons, choke-point placement, compile-error enforcement, honest cost clauses). The open decisions (D-4, D-5) go to the maintainer, not into a workstream. If a decision gets taken, update the corresponding initiative briefs and `FINDINGS-INDEX.md` statuses to match — the corpus is meant to stay true as the repo moves.
