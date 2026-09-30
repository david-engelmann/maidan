# Open work

The one live list of what is being built, what comes next, and what is waiting
on a decision. Last reconciled against `main` at `4b1cbb9e` (2026-09-30).

**The rule.** The PR that changes an item's state edits its row here, in the
same PR. A shipped item is deleted, not struck through: its record is the
CHANGELOG entry and the PR. A wrong row is corrected in place, never answered
by a new row beside it. Nothing in **Now** or **Next** is marked done.
`scripts/check-open-work.sh` (CI job `open work`, not required) fails when a
PR listed under **Now** has merged or the reconciliation stamp above is not on
`main`; a red run means this file needs reconciling.

History (the dated programs, audits and dispositions this file carried until
2026-09-29) is in [Open Work 2026-08-25 to 2026-09-28](archive/Open%20Work%202026-08-25%20to%202026-09-28.md).
Direction is in [Roadmap](Roadmap.md); what shipped is in
[Capabilities](Capabilities.md) and `CHANGELOG.md`.

## Now: in flight

Merge order matters where one PR builds on another; the column says so.
Rows without a PR number are being built in a lane and get one when opened.

| PR | What it does | Closes | Merge after |
|---|---|---|---|
| #1127 | Rotate a token from the Session and Tokens tabs; `/me` returns `token_id`; the Session tab stops saying a token acts as any member | The rotation half of thread 2a | — |
| #1142 | A pasted token is exchanged for the `HttpOnly` session, so no token stays in `localStorage`; unsafe session requests from another origin are refused; `csrf_secret` is dropped | Thread 2b; the `localStorage` risk | #1127 (reconcile with the session-audit methods #1136 added; rotation ends a session, so the Session tab re-exchanges the successor) |
| #1137 | A2A `ListTasks` decides access in the store query, so the query count does not grow with hidden tasks | ListTasks pushdown | — |
| #1140 | `secret://` references are substituted on automation and A2A push egress, against a per-workspace allowlist | Secret substitution on every egress path | — |
| #1100 | F-48 Tier 1: one `store_delegations!` list for both backends | The store delegation duplication | Everything above that adds a `Store` method |
| (lane T) | A foreign member is refused like an unknown one everywhere; `authority_audit_contract` scans past `#[cfg(test)]`; JSON-RPC `-32600` for a non-request; stdio stays silent on notifications | Lane findings, 2026-09-30 | — |
| (lane U) | TCK numbers attributed to the right check; `llms.txt` lease and create-thread wording; enums on `transition_thread`; see-also on the wait tools; `waiting.spec.ts` asserts something | Thread 34–38, part of 50 | — |

## Next: ranked

Ranked by what an agent or a person relying on the room loses without it. An
item enters this table with acceptance criteria and a check that shows it is
still open. "Thread" numbers refer to the 2026-09-29/30 enhancement thread,
whose dispositions are recorded below.

| # | Item | Size | Acceptance criteria | Open because | Depends on |
|---|---|---|---|---|---|
| 1 | **UI fixes found by the 2026-09-30 QA pass** (thread 30–33, 42, 53–55, 57) | XS–S each | A keyboard focus reveals the pin toggle; inputs are 16 px so iOS does not zoom; a palette jump to another channel's thread selects that channel; the first-run card says what a workspace id is and where to get one, and the empty-channel help names the board's "Add task" input; clicking a message offers an edit affordance and opens the editor it fills; `.chrome-idle` and `.brand-sub` are removed; a successful token clears the earlier error text; a failed post keeps the text and says why; a channel refresh against a dead server shows the friendly unreachable message; opening a DM selects it and names its members | Each reproduced by the QA pass on `1e612cf8`; see the thread entries | The UI stack (#1123, #1127, #1135, #1142) |
| 2 | **Write paths for a signed-in person** (thread 29) | S | An OIDC session can edit a message, upload and paste an artifact through `/ui/api`; four proxy routes (thread GET, review-status GET, message PATCH, workspace GET); bearer-only routes answer a session with a sentence, not an HTTP code; `apiReadPath()` beside `apiWritePath()` | Edit, upload and paste call `requireTokenForWrite()`; #1142 lets a token's session reach bearer routes, but an OIDC session still cannot | #1142 |
| 3 | **The UI improvement specs** (thread 48) | S per surface | Signed in, the header is an identity pill plus Change and Sign out; the drawer groups its tabs (yours, observe, operate), starts closed and drops the Work tab; messages get a hover toolbar with keyboard parity and inline edit; the Session tab shows an identity card; the board keeps a one-line badge legend; every empty state names the next action; phone layout last | The QA pass graded the drawer B+, session management C and error states D | #1–2 |
| 4 | **Split the UI into typed modules** (thread 27, superseded by 49) | L, in steps | The CSS and JS leave `index.html` as ES modules, one per surface, with JSDoc types checked by `tsc --noEmit --checkJs` in CI; files served from the binary (`include_str!` or `rust-embed`), so `cargo build` needs no Node; a typed client module generated from `/openapi.json`; the DM and group-DM code collapsed into one module (thread 39); one `api()` fetch wrapper and one feedback surface (thread 40); Playwright specs unchanged | One 5,000-line file every UI change lands in, and no type checking | #3 |
| 5 | **A restrictive CSP for `/ui`** (thread 28) | S | `/ui`, `/ui/` and `/ui/static/*` send `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; connect-src 'self' https: wss:; font-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'`; a test asserts the header and that the page loads under it | `app.rs` serves the page with no CSP; inline script and styles would need nonces until #4 moves them out | #4 |
| 6 | **Budget stops leave the thread claimable** | S | A thread past any budget is not handed out by `claim_next` until its budget is raised or reset, with a test per budget kind | Found by #1139: after a stop the thread stays claimable, so each new claim fails in turn | — |
| 7 | **Freeze follow-ups** | S | A freeze emits `ThreadAssignmentChanged` for each claim it releases; a freeze can carry an expiry | #1124 releases claims with a raw `UPDATE` and records only a count; freezes last until an unfreeze | — |
| 8 | **Per-workspace fairness by default** (thread 51) | S | `MAIDAN_WORKSPACE_RATE_LIMIT_MAX` has a built-in default, as the global limit does, and `0` turns it off | The per-workspace limit is off unless set (`rate_limit/mod.rs`, Production) | — |
| 9 | **Per-workspace retention** (thread 51) | M | A workspace can set its own retention for messages, events and deliveries within the instance's bounds; a two-tenant test | Retention is global env only (`retention.rs`) | — |
| 10 | **Playwright and unit coverage** (thread 50) | M | Specs for prefs, slash commands, delivery replay, token mint and revoke, and DMs; unit tests for the auth-routing helpers and the error parser; a coverage checklist in `ui-tests/README.md` | None of those have a spec today | #4 for the unit tests |
| 11 | **Web push from the browser** (thread 23, second half) | M | The `/ui` registers a push subscription; a failed push is retried durably | The `/ui` has no subscribe flow; a failed push is not retried | — |
| 12 | **Projector depth** | M, S, S | GitHub App JWT exchange and Check Runs; a `thread_id` index on Slack channel links; a signal when a linked PR closes unmerged | `github.rs` takes a PAT or installation token only; `0051_slack_channel_links.sql` indexes `workspace_id` only | — |
| 13 | **The money shots** (thread 12, 47) | M | A seeded capture workspace (`capture_seed` example on the `ui_test_server` pattern) and a Playwright script that captures the board, a thread with two agents and a review, and Needs you; images in `docs/assets/` and the README; a retake rule for board-touching PRs | No UI image exists | #3 |
| 14 | **Verification depth, what remains** | M each | Kani proofs of capability containment and cursor arithmetic; the nightly store mutation job sharded so it finishes | Fuzzing and auth/bus mutation landed (#1125); `cargo mutants --list` counts 4,642 store mutants against a 90-minute job, and `continue-on-error` hides the timeout | — |
| 15 | **Supply chain, what remains** | S | `cargo-vet` over the root lockfile | SBOM attestation and OSV landed (#1119); the TypeScript and Python SDKs have no lockfiles to scan | — |
| 16 | **Partition the append-only tables** | L | Per-table autovacuum and range partitioning of the event log, audit and delivery tables, with retention as `DROP PARTITION` | The tables are unpartitioned, so retention is row-by-row `DELETE` and vacuum works the whole table | — |
| 17 | **Adoption** | S–M each | An OpenHands claimant recipe; provider recipes I2–I6 ([Providers](Providers.md)); the official third-party SDKs run as clients in CI | Retro 418's carry-forward | — |

**Measure before scheduling:** the per-response query count (F-51); whether
the workspace context pack filters before it builds; the search deny-set
query; the board's full re-render on large channels (thread 17); `catch_up`
page cost as the event log grows (thread 18).

**Found by the 2026-09-30 lanes, small enough to fold into the nearest PR:**
the store's `*_audited` methods do not check that an audit's scope matches the
workspace they write in (#1134); the request-layer `mutation` row stamps the
token's workspace, not the path's, for a bypass caller (#1134); a receiver that
echoes a secret back in a slash response puts it in `metadata.slash_response`
(#1140, documented); the retry budget is per replica and does not bound
first-attempt floods (#1122, documented); the 0124 backfill ordering has no
test of its own (#1132); scripts that build into `./target` ignore
`CARGO_TARGET_DIR` (#1129).

## Later: by program

One line per item, promoted into Next when it gets acceptance criteria.

- **R — resilience:** chaos steady-state checks; the `maidan` CLI refusing unknown `MAIDAN_*` variables as the server does (#1107); native TLS on the gRPC listener (#1114 requires an acknowledgement instead).
- **V — verification:** stateful model tests beyond the claim lease; Kani over idempotency keys and FSM totality.
- **D — Postgres operations:** `rebuild-derived --verify` (re-fold the notification ledger, follow counts and inbox cursors from the log, with a drift metric); XID-age, bloat, WAL and replication-slot metrics; a pool-topology guide; a UTF-8 pin.
- **B — protocol conformance:** snapshot tests over normalized MCP, A2A, OpenAPI and audit shapes, so wire drift is a reviewed diff.
- **S — security:** `authority_audit_contract` over every crate and example with its two carve-outs (`maidan init`, the Playwright harness) named, so a call outside the two watched crates cannot slip past (thread 14b); threat-regression gates (each Threat-Model row names its test) and a per-PR threat-model delta; token TTLs with DPoP or mTLS binding; audience-bound step-up auth for sensitive MCP tools; an over-permission scan of the MCP surface; a provenance label on content from other tenants, tools and external sources; `miri` over the `unsafe` in dependencies.
- **E — compliance:** read-time PII masking on messages, content blocks and audit metadata; scheduled key rotation and a key-age policy.
- **O — observability:** a buffered, redacted collector pipeline with a tested loss contract; opt-in `tokio-console`; a named p99 on the fan-out paths; operator views.
- **F — search quality:** reranking (RRF or a cross-encoder); an HNSW recall evaluation (the `m`/`ef` knobs are pgvector defaults); a query-embedding cache; a search evaluation gate over `relevance_eval.rs`; a ParadeDB comparison; facet search by reference kind, result kind and event kind.
- **G — developer experience and governance:** a fitness-function catalog; semver checks for the SDKs; a glossary-grounding lint; active benchmarking with k6; a devcontainer, pipeline parity, typed config, a cross-build matrix and an S3 test double.
- **U — the web UI:** search, workspace context and the explorer adopting the store's SQL thread-access predicate (#1137) instead of checking DM access after the query; design tokens; a code editor, log stream and DAG view; a reference-graph pane over the backlink index; an interactive OpenAPI reference; recorded demos; one-click token mint in Connect; a member last-active endpoint; cross-channel palette search; delivery and allowlist panels.
- **Federation** (when federation gets real multi-peer use): a per-peer egress allowlist in place of the whole-guard `MAIDAN_ALLOW_PRIVATE_EGRESS` bypass, which production already refuses (thread 10); a versioned envelope contract; cross-peer causal order; verifiable-credential peer trust.
- **Parked evaluations** (revisit on a concrete trigger): OPA/ABAC; Tantivy; compile-time SQL; SOPS; a CI egress allowlist; an in-flight-run drain signal; allocator and zero-copy micro-evaluations; an MCP registry policy.
- **L — launch** (the maintainer's call): positioning, the public site and funnel, the in-browser playground, the trust page, comparison and migration pages, and the open-core boundary (the room stays open source; the hosted control plane is the paid tier). A hosted service waits for three things together (thread 52): teams asking for hosted who will not self-host, operations bandwidth, and per-tenant key custody designed before the first stranger's data lands; paid self-hosted tiers (license key, SSO, SCIM, audit exports, support) are the nearer option.
- **Migrations on a live fleet** (thread 51): a written expand/contract policy for schema changes when more than one server version runs at once.

## Decisions pending the maintainer

| Question | Options | Recommendation | Blocks |
|---|---|---|---|
| Require branches to be up to date before merging (F-43)? | Turn on `strict`; or keep it off | Turn it on. Until then the merge discipline in Process below stands in for it | Nothing, but every stale-base merge is a risk |
| Should a browser session get `thread:transition`? | Yes, so a person can start review and close from the UI; or keep it token-only | Yes, under the same separation-of-duties checks. With #1142 a token's session already carries the token's own capabilities, so this is only about OIDC sessions, which carry a fixed five | The UI's review actions for OIDC users |
| Should a token's session reach every bearer route? | As built in #1142 (except MCP); or confine it to `/ui/api` | As built: the page calls about 20 bearer routes directly; the session re-resolves the token's authority on every request, ends with the token, and refuses unsafe requests from another origin | Merging #1142 |
| When to publish the SDKs (`sdk-*` tags)? | 0.2.0 from before #1129 and then 0.3.0; or 0.3.0 only | 0.3.0 only: typed results and errors (#1129) are on `main`, and nobody depends on 0.2 | SDK users |
| How long do read notifications and the usage ledger live? | A retention knob for each; or keep forever | Read notifications: `MAIDAN_RETENTION_NOTIFICATIONS_DAYS`, off by default. Usage ledger: keep, since it is a billing record | Unbounded growth of `maidan_notifications` |
| Squash commits: keep the commit messages, or use the PR body? | `COMMIT_MESSAGES` (today); `PR_BODY` | `PR_BODY`, so each squash commit carries its retro, as Decisions says it should | Nothing |
| Does `max_wall_secs` count every way a claim ends? | Lapsed leases only (#1139); or also releases, reassignments, freezes and budget stops | Every end, so the budget means the thread's total worked time; about ten store paths per backend | The meaning of the wall budget |
| What does a SCIM group grant? | Nothing (#1133 records membership only); channel access; capability templates | Nothing until an operator asks; then channel access first | IdP-driven access |
| Is a SCIM `userName` unique regardless of case? | Case-insensitive, with a `lower(handle)` unique index for every member; or case-sensitive as today | Case-insensitive: the filter already matches that way (#1133), so `Alice` and `alice` can both exist and both match | Handles that differ only in case |
| Per-workspace soft delete for artifacts? | A per-reference tombstone; or erase only, as today | Erase only until a workspace needs to hide without deleting; the shared `tombstoned_at` column is never written and could be dropped | Nothing today |
| Public launch, a hosted playground, a paid tier | — | See Later, L | Program L |

External setup that blocks rows, and is not a decision: live Slack and GitHub
apps for projector tests, a provider key for the Goose recipe, and Jev access
for J-01.

## Recently decided

Each links its record. Entries roll off after about a month.

| Date | Decision | Record |
|---|---|---|
| 2026-09-30 | A blank `/ui` opens on a first-run card; the discovery document says which sign-in paths exist; Sign out forgets the token | #1123 |
| 2026-09-30 | A claim freed on a lapsed lease is charged its worked time against `max_wall_secs` | #1139 |
| 2026-09-30 | A verdict is a `ReviewSubmitted` event; a change request notifies the last worker to hold the thread | #1132 |
| 2026-09-30 | A member id naming no member, or another workspace's, is a 404; sign-in and sign-out are audited; A2A reads are not recorded as changes | #1136 |
| 2026-09-30 | Every audit row names a workspace (or the instance), stamped at write; a hold keeps only its own workspace's rows | #1134 |
| 2026-09-30 | Every HTTP operation is classified as reading or changing state, and CI checks it | #1121 |
| 2026-09-30 | A SCIM group records membership and grants nothing; a `userName` rename keeps the member's id | #1133 |
| 2026-09-30 | Stateless MCP subscriptions live in a shared, expiring table, so any replica delivers them | #1131 |
| 2026-09-30 | Retries to one host share a per-host token bucket; a refused retry is deferred, never dropped | #1122 |
| 2026-09-30 | A SQLite backup is a `VACUUM INTO` snapshot; a restore deletes the old `-wal` and `-shm` | #1126 |
| 2026-09-29 | A misspelt `MAIDAN_*` variable refuses boot | #1107 |
| 2026-09-29 | A2A pushes go over https; plaintext gRPC off loopback needs an acknowledgement | #1114 |
| 2026-09-29 | A token can be rotated: a new secret for the same authority, the old one revoked in the same transaction | #1116 |
| 2026-09-29 | Release tags wait; they are the maintainer's call, not a per-cluster step | [Operations](Operations.md) |
| 2026-09-29 | Delivery retention never prunes a pending row or a dead letter | #1108 |
| 2026-09-29 | Every egress request times out (5 s to connect, 10 s in all); an A2A task holds at most 10 push configs | #1109 |
| 2026-09-29 | A WebSocket subscription announces only its own member | #1112 |
| 2026-09-28 | Withdrawn messages are crypto-shredded; the server refuses to start without a KEK | #1063, #1064 |
| 2026-09-25 | A change request reopens the thread and dismisses approvals | #1054 |
| 2026-09-25 | A legal hold keeps withdrawn words; holds are per matter | #1056, #1057 |
| 2026-09-23 | D-A: authority changes write their audit row in the same transaction | [Decisions](Decisions.md) |
| 2026-09-23 | D-B: a per-workspace grant ceiling, 90 days by default | [Decisions](Decisions.md) |
| 2026-09-23 | D-C: delegated capabilities are a two-way intersection; members carry none | [Decisions](Decisions.md) |
| 2026-09-23 | Approvals may be borrowed, never self-approved | [Decisions](Decisions.md) |
| 2026-09-23 | F-54: no `/v1` and no compatibility promise before 1.0 | [Decisions](Decisions.md) |

## Known risks

| Risk | Mitigation | Residual | Watched by |
|---|---|---|---|
| A pasted bearer token lives in `localStorage` | Tokens are capability-scoped, revocable and rotatable (#1116); #1093 escapes the page's `innerHTML` writes; #1142 (in flight) exchanges the token for an `HttpOnly` session | Until #1142 merges, a script injected into the page can read the token; after, it can act as the session while the page is open but cannot take the credential away (Threat-Model T17) | #1142, then Next #5 (CSP) |
| Admin merges while branch protection is not strict (F-43) | The merge loop reads the checks of the PR's exact head commit, requires all eight required checks there, merges with `--match-head-commit`, and first builds the PR merged onto current `main` and runs the static contracts | A break that only a full test run on the merged tree would catch; `main`'s own CI catches it after the merge | `main` CI; Process below |
| Events delivered at most once on the optimistic path | Transactional outbox, quarantine and replay; opt-in `at_least_once` delivery per `consumer_id` | A client that does not opt in can see a gap or a duplicate | `maidan_outbox_quarantined`, `MaidanOutboxQuarantined` |
| `AUTH_DISABLED` or bootstrap left on | Both refuse boot without an explicit acknowledgement, and always in production; hardened builds strip the path | A dev binary exposed to a network | Boot checks |
| The embedding indexer falls behind | Retries, a repair sweep, and `/health/ready` degraded past `INDEXER_STALE_SECS` | `INDEXER_STALE_SECS` is off by default | `maidan_indexer_last_event_age_seconds` |
| `hash-v1` embeddings are not semantic | A boot warning; `openai-compatible` for real use | A deployment that ignores the warning | Boot log |
| The Postgres bus listener drops | Reconnect with back-fill from the log | Delay while it reconnects | `/health/ready`, `maidan_bus_lag_total` |
| `RUSTSEC-2023-0071` (`rsa`) | Ignored with a reason; RS256 verification only | Clears with `openidconnect` 5 | `cargo-deny` |
| One tenant saturates a shared instance | Per-token and per-workspace rate limits, the in-flight ceiling, statement and lock timeouts, egress timeouts | No CPU or IO isolation between tenants | `maidan_http_shed_total`, `MaidanLoadShedding` |

## Won't do

| Item | Why | Record |
|---|---|---|
| Postgres row-level security | Isolation is enforced in the application and proven per route by `tenant_isolation_e2e` | [Decisions](Decisions.md) (`v216.0.0`) |
| A workflow engine (Restate, Temporal) | The outbox, DAG and scheduler already give the semantics | Decisions |
| MCP argument defaulting (`author_id` from the caller); hoisting the context assembler into `maidan-router` | A semantic change to the post path for little gain (`whoami` covers self-discovery); multi-crate surgery when the shared fold already lives in `maidan_types` | Cluster 349 ([archive](archive/Open%20Work%202026-08-25%20to%202026-09-28.md)) |
| The flagship arc's optional tail | Declined at `v331.0.0` | Decisions |
| F-48 Tier 3, F-53, F-33, J-06 | Declined on the merits | [archive](archive/Open%20Work%202026-08-25%20to%202026-09-28.md) |
| Batched `pg_notify`; the ACP IDE bridge; multi-region active-active; a CRDT log | Out of scope | [Roadmap](Roadmap.md) |
| The A2A TCK's scenario groups | They need a scripted agent, which Maidan is not | `scripts/a2a-tck/exclusions.txt` |
| Wildcard egress selectors; MCP twins of the egress allowlist | An allowlist must name ids, so a wildcard defeats it; the result-delivery allowlist is operator policy, set over REST by `token:admin` (the per-workspace secret-egress allowlist in #1140 is a separate list with MCP tools) | [Result Delivery](Result%20Delivery.md) |
| J3: `ttlMs`, `cacheScope`, `server/discover`, `request_client` | Optional MCP `2026-07-28` features no client relies on | [Retro 303](https://github.com/david-engelmann/maidan/blob/main/docs/Retros/Cluster%20303.0.md) |
| Retroactively tagging `v411.0.0` | It would rewrite a recorded history; the record says "never tagged" | Thread 22, #1078 |

## Process

**Merge discipline while branch protection is not strict (thread 43).** A PR
merges when every one of the eight required checks has passed on its exact
head commit and nothing else on that commit is queued, running or failed; the
merge names that commit (`gh pr merge --match-head-commit`), so a push in
between makes GitHub refuse it. Before merging, the PR is merged onto current
`main` locally and built, and the static contracts run on the result
(`attribution_scope_contract`, `authority_audit_contract`,
`http_operation_kinds_contract`, `docs_numbers_contract`,
`env_registry_contract`, `ui_js_contract`, `openapi_well_formed`, the event-kind
and lexicon contracts, `migration_register`, `backend_parity`). Stacked
branches rebase in dependency order, replaying only their own commits.
Two merges broke this on 2026-09-30 and are why the rules read as they do:
#1124 merged while `gh pr checks` still showed the previous commit's runs, and
#1133's last commit merged two seconds after a push, before GitHub reported the
new head. `main`'s CI passed on #1124's merge; #1133's is recorded in its PR.

**UI changes (thread 42, 46).** The PR template's `/ui` checklist is run by
hand before a UI PR is opened: the token path and the OIDC path, keyboard only,
a 390 px viewport, and every new string re-read.

## Enhancement thread 2026-09-30: dispositions

The 2026-09-30 entries (28–57) and the supersessions of 27, 28, 29 and 35.
Entries marked done on `main` since the thread's `1e612cf8` snapshot are noted.

| Thread | Item | Disposition | Where |
|---|---|---|---|
| 28 | CSP for `/ui` | **Adopt**, with the supersession's exact header, after the module split | Next #5 |
| 29 | Write paths for session users | **Adapt**: #1142 makes a token's session reach bearer routes; what is left is the OIDC session, plus the four proxy routes and honest refusals | Next #2 |
| 30 | Hover-only controls, 16 px inputs | **Adapt**: the reaction button already reveals on `:focus-within`; the pin toggle and input sizes remain | Next #1 |
| 31 | Palette jump leaves the sidebar on the old channel | **Adopt** | Next #1 |
| 32 | First-run copy | **Adapt**: #1123 replaces the "OIDC" label and adds token help; the workspace-id help and the stale "New thread in the sidebar" text remain | Next #1 |
| 33 | Message click affordance | **Adopt** (same as 7) | Next #1 |
| 34 | TCK numbers disagree | **Adopt**: the two sets come from two checks (the TCK's pytest summary and the walkthrough), and the comment conflated them | Lane U |
| 35 | MCP has no create-thread tool | **Adopt** the supersession: say REST-only by design in `llms.txt` and the catalog | Lane U |
| 36 | `llms.txt` says an unleased claim never lapses | **Adopt**: every claim is leased, 600 s by default | Lane U |
| 37 | Enums on string inputs | **Adapt**: `transition_thread.action` gets one; `set_delivery_mode.mode` already has one; `cast_vote.kind` is free text by design (emoji) | Lane U |
| 38 | Wait families look alike | **Adopt**, as see-also sentences | Lane U |
| 39 | DM and group-DM duplication | **Adopt**, folded into the module split | Next #4 |
| 40 | One fetch wrapper and one feedback surface | **Adopt**, folded into the module split | Next #4 |
| 41 | A checked API client for the UI | **Adopt**, as the generated typed client in the module split | Next #4 |
| 42 | Dead CSS and a lint | **Adapt**: remove `.chrome-idle` and `.brand-sub`; the lint waits for the split, when the CSS is a file | Next #1; Next #4 |
| 43 | F-43 and merge order | **Adopt the discipline**, which is written in Process; turning on `strict` stays the maintainer's; the "fail any PR whose merge-base is older than 24 h" CI rule is **rejected**, because it would rerun every open PR's full CI after each merge on a runner pool that is already the bottleneck, and the pre-merge build on current `main` catches the same breaks | Process; Decisions pending |
| 44 | Open Work as a CI contract | **Adapt**: `scripts/check-open-work.sh` fails when a PR under Now has merged or the stamp is not on `main`; a per-row `check:` line is **deferred** until the table is stable enough to be worth scripting | This file; CI `open work` |
| 45 | `thread:transition` for browser sessions; one owner for the browser-authority chain | **Adopt**: the decision stays the maintainer's, with the recommendation above; the chain is warn (#1123), rotate (#1127), exchange (#1142, which also drops `csrf_secret`), with one owner (the coordinating agent); on rotation the session ends and the page exchanges the successor, rather than migrating the session row | Now; Decisions pending |
| 46 | `data-ui-version` never versions | **Adapt**: delete the marker and its eight test assertions rather than pin it, since a pin would make every UI PR conflict on one line | Next #1 |
| 47 | The money shots | **Defer** until the UI improvement specs land, so the images show the fixed UI | Next #13 |
| 48 | UI improvement specs | **Adopt**, one surface per PR, after the in-flight UI stack; first run, Sign out and rotation are already in #1123, #1127 and #1142 | Next #3 |
| 49 | Split as TypeScript from the start | **Adapt**: ES modules with JSDoc types checked by `tsc --noEmit --checkJs`, not TypeScript source compiled by esbuild, so the release build and `cargo install` need no Node; the typed client and the module map stand | Next #4 |
| 50 | Playwright and unit gaps | **Adopt**: the `waiting.spec.ts` fix now, the rest with the split | Lane U; Next #10 |
| 51 | SaaS-agnostic hygiene | **Adopt**: per-workspace fairness by default and per-workspace retention; the migration policy goes to Later | Next #8, #9; Later |
| 52 | Do not build a hosted service yet | **Adopt** as recorded | Later, L |
| 53 | Stale token error after a success | **Adopt** | Next #1 |
| 54 | A failed post is lost silently | **Adopt** | Next #1 |
| 55 | Channel refresh shows `TypeError` | **Adopt** | Next #1 |
| 56 | No sign-out for a token user | **Done in #1123**: Sign out is offered to a token and forgets it | #1123 |
| 57 | Opening a DM does not select it | **Adopt** | Next #1 |
| 27 → 49 | Module split | See 49 | Next #4 |
| 28 (superseded) | CSP header and sequencing | See 28 | Next #5 |
| 29 (superseded) | The 19-site table | See 29 | Next #2 |
| 35 (superseded) | REST-only doc fix | See 35 | Lane U |

Done since `1e612cf8`, per the thread's own list: 3 (#1118), 13, 14a, 19, 20,
22, 23 (#1113; 23's numbers corrected by lane U). The 2026-09-29 dispositions
below stand, except where a row above supersedes them.

## Enhancement thread 2026-09-29: dispositions

David's auditor re-verified each item five times against `main` and freshened
it to `70a6d78b`. Each item's disposition, and where it lives now.
Items 5, 21 and 25 were struck by the thread itself (false premises) and are
not re-filed.

| Thread | Item | Disposition | Where |
|---|---|---|---|
| 1 | Inline errors instead of `alert()` | **Adopt**, P0 | Done, #1117 |
| 2a | Warn that the token is stored; surface rotation | **Adopt**, P0 | #1123, #1127 |
| 2b | httpOnly cookie sessions for the UI | **Adapt**: the session cookie already exists and is `HttpOnly; SameSite=Lax` (`session/mod.rs`). The work is exchanging a pasted bearer for it, and verifying or removing the unread `csrf_secret` | #1142 |
| 3 | One primary thread surface | **Adopt**, P0 | Done, #1118 |
| 4 | Inline attachment previews | **Adopt**, with `nosniff` and a sandboxing CSP on artifact bytes | #1135 |
| 6 | Header cleanup | **Adapt**: the residual after #1086's Connect sheet | #1123 |
| 7 | Edit affordance on messages | **Adopt** | Next #1 |
| 8 | Reorganize the tools drawer | **Adapt**: the palette (#1086) is the fast path; group the tabs and drop the Work tab's copy of the board | Next #3 |
| 9 | Dead `#live-*` CSS | **Adapt**: the first block is not dead (see below); merge the two into one rule per element instead | Done, #1117 |
| 10 | Per-peer egress allowlist | **Defer** until a real federation peer; production already refuses the bypass | Later, Federation |
| 11 | Token rotation endpoint | **Adopt**, with 2a | Done, #1116 |
| 12 | Board screenshot in the README | **Adopt**, after the P0s so the image shows the fixed UI, captured by a script | Next #13 |
| 13 | The recording, diagram and session in the docs site | **Adopt** | Done, #1113 |
| 14a | Cluster 413 overstates the D-A guarantee | **Adopt** | Done, #1113 |
| 14b | Make the D-A guarantee tighter | **Adapt**: widen `authority_audit_contract` to every crate with the two carve-outs named; pinning, not a hole | Later, S |
| 15 | gRPC TLS by default | **Adapt**: refuse plaintext off loopback without an acknowledgement now; native TLS later | Done, #1114; Later, R |
| 16 | A2A push accepts `http` | **Adopt** | Done, #1114 |
| 17 | Board render for large lists | **Defer**: measure first | Measure before scheduling |
| 18 | `catch_up` pagination at scale | **Defer**: it is keyset-paginated with a limit, so flat by construction; measure before changing | Measure before scheduling |
| 19 | README says 165 tools | **Adopt**, with a contract test | Done, #1113 |
| 20 | Conformance jobs required, or a drift policy | **Adapt**: the drift policy is written (Conventions); promotion is the maintainer's | Done, #1113; Decisions pending |
| 22 | Release checklist step for tagging | **Adapt**: tag the day the record merges or relabel it never-tagged; no retroactive tag | Done, #1113 |
| 23 | Record the TCK run summary | **Adopt** | Done, #1113 |
| 24 | First-run onboarding | **Adapt**: the residual after #1086 | #1123 |
| 26 | Narrow-viewport and keyboard QA | **Adapt**: #1093 landed; its residue is the keyboard and 390 px pass in the UI specs | Next #3 |
| 27 | Split `static/index.html` | **Adopt**, after the P0s; superseded by 49 | Next #4 |

**Sequencing, as it ran.** The P0s went first: inline errors (#1117) and one
thread surface (#1118) merged on 2026-09-30, and the token chain is #1116
(merged), #1123, #1127 and #1142.

**Where the thread was wrong.** Item 2b assumed the UI had no cookie session:
`crates/maidan-server/src/session/mod.rs` already issues `HttpOnly;
SameSite=Lax` (and `Secure` when configured) cookies for OIDC sign-in, and each
session stores a `csrf_secret` that no handler reads, so `SameSite=Lax` is the
CSRF defence in force. Item 9 took the first `#live-panel` / `#live-toolbar` /
`#live-feed` block for dead because a later block restyles the same
elements, but the cascade is per property: the later block never sets the flex
layout or the feed's colours, font, overflow and wrapping, so removing the
first would break the Live bar. Everything else checked out.
