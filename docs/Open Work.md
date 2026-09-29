# Open work

The one live list of what is being built, what comes next, and what is waiting
on a decision. Last reconciled against `main` at `70a6d78b` (2026-09-29).

**The rule.** The PR that changes an item's state edits its row here, in the
same PR. A shipped item is deleted, not struck through: its record is the
CHANGELOG entry and the PR. A wrong row is corrected in place, never answered
by a new row beside it. Nothing in **Now** or **Next** is marked done.

History (the dated programs, audits and dispositions this file carried until
2026-09-29) is in [Open Work 2026-08-25 to 2026-09-28](archive/Open%20Work%202026-08-25%20to%202026-09-28.md).
Direction is in [Roadmap](Roadmap.md); what shipped is in
[Capabilities](Capabilities.md) and `CHANGELOG.md`.

## Now: in flight

| PR | What it does | Closes | Waiting on |
|---|---|---|---|
| #1086 | A command palette, a Connect an agent sheet with copyable MCP config, empty states that say how work arrives | Part of thread items 6, 8 and 24 | CI |
| #1093 | The `/ui` audit: axe-clean contrast and keyboard, one-column phone layout, resilient Live, error states | Part of thread item 26 | #1086 |
| #1097 | `ClaimUnacknowledged`: a leased claim nobody acknowledged is reported (migration 0120) | The second half of eager reclaim | #1107 (registers `MAIDAN_CLAIM_ACK_TIMEOUT_SECS`) |
| #1098 | Review and land-gate verdicts keep their history (migration 0121) | Decision history | CI |
| #1100 | F-48 Tier 1: one `store_delegations!` list for both backends | The store delegation duplication | Last in the queue, since it touches every `Store` method |
| #1107 | A misspelt `MAIDAN_*` variable refuses boot, with a did-you-mean (F-52) | F-52 | CI |
| #1113 | Six docs fixes from the enhancement thread | Thread items 13, 14a, 19, 20, 22, 23 | CI |
| #1114 | A2A pushes go over https; plaintext gRPC off loopback needs `MAIDAN_A2A_GRPC_PLAINTEXT=1` | Thread items 15 and 16 | #1107 (registers the new variable) |

## Next: ranked

Ranked by what an agent or a person relying on the room loses without it. An
item enters this table with acceptance criteria and a check that shows it is
still open. "Thread" numbers refer to the 2026-09-29 enhancement thread, whose
dispositions are recorded below.

| # | Item | Size | Acceptance criteria | Open because | Depends on |
|---|---|---|---|---|---|
| 1 | **Inline errors instead of `alert()`** (thread 1, with 9) | M | One inline and toast error component; every `alert(` in `static/index.html` replaced; `ui_js_contract` fails on a new `alert(`; the dead first `#live-panel` / `#live-toolbar` / `#live-feed` CSS block removed | 39 blocking native dialogs in `static/index.html`, all validation guards | #1093 |
| 2 | **One thread surface** (thread 3) | M | The board is the thread surface; the sidebar list becomes a compact switcher (or is removed), so a thread is drawn once; `loadThreads` renders one surface | `loadThreads` renders `#thread-list` and `#board` from the same fetch, so every thread appears twice | #1093 |
| 3 | **Bearer tokens in the browser: warn, rotate, stop storing** (thread 2a, 11 and 2b) | M, then M | `POST /tokens/{id}/rotate` returns a new secret and revokes the old one, audited in the same transaction (D-A), with an MCP twin; the `/ui` warns that a pasted token is kept in this browser and offers rotation in the Session tab; then a pasted bearer is exchanged for the existing `HttpOnly; SameSite=Lax` session cookie, so no token sits in `localStorage`, and the session's `csrf_secret` is either verified on `/ui/api` writes or removed | `static/index.html` keeps the bearer in `localStorage` (`maidan_token`) with no warning; `routes/token.rs` has no rotation; the session stores a `csrf_secret` nothing reads | — |
| 4 | **Workspace-wide `claim_next`** | M | `POST /workspaces/{wid}/threads/claim-next` and an MCP twin hand out the oldest ready thread across every channel the caller can read, with the channel route's filters (open state, skills, blocked reasons, DAG readiness, claim gates), its fencing token and the default lease; a private channel's threads go only to its members; a two-tenant test | The only route is `/channels/{cid}/threads/claim-next` (`app.rs`), so a workspace agent polls every channel | #1097 |
| 5 | **Review-loop signals** | S | A change request notifies the thread's last worker (a `changes_requested` notification), and every verdict appends a `ReviewSubmitted` event, in the lexicon | `notification_router.rs` has no change-request arm; an approval emits no event; a reopened worker learns of it only by polling | #1098 |
| 6 | **The wall-clock budget is charged when the reaper frees a claim** | S–M | A claim freed by the reaper charges the time worked against `max_wall_secs`, and one past it stops with `ClaimFailed`, as `report_usage` would | The budget is enforced only inside `report_usage`, which a hung agent never sends | — |
| 7 | **First run and the header** (thread 24 and 6, after #1086) | M | A blank page walks a new user from API base and workspace to a token or OIDC login and a connected socket; the raw API base / Workspace / Token fields move behind a connection popover; "Out" reads "Sign out" | The header paste fields are the whole onboarding; #1086's Connect sheet covers the agent side only | #1086 |
| 8 | **Inline attachment previews** (thread 4) | M | Image artifacts render inline and every artifact shows its filename; artifact bytes are served with their content type, `X-Content-Type-Options: nosniff` and a sandboxing CSP, so inline rendering cannot run script | The artifact card is a link with a 16-character SHA prefix; the attach message is "Attached artifact" and a 12-character prefix | — |
| 9 | **A shared egress retry budget** | M | Retries across the webhook, automation, egress and mail workers draw on one budget per destination host; past it, retries are deferred, not dropped; `maidan_egress_retry_deferred_total`; a test where a recovering destination receives at most the budget | Each worker retries on its own backoff, so a destination that recovers takes the whole backlog at once | — |
| 10 | **Every audit row belongs to a workspace** | S–M | `maidan_audit.workspace_id`, stamped at write and backfilled; `GET /workspaces/{wid}/audit` reads it; a legal hold exempts only its workspace's rows from pruning; a two-tenant test | Audit rows carry no workspace, so a hold freezes audit pruning for the whole instance (`retention.rs`), and a workspace's audit view omits rows with no actor: app-token mints (`app_oauth.rs`), SCIM provisioning (`scim.rs`), result-delivery attempts (`egress_worker.rs`) | — |
| 11 | **UI polish after the P0s** (thread 7, 8 and 26) | S each | Hovering a message shows an edit affordance, and clicking it opens the editor it fills; the "More tools" drawer groups its 16 tabs and drops the Work tab's copy of the board, with the palette (#1086) as the fast path; a keyboard-only and 390 px pass over whatever #1093 does not cover | Clicking a message silently fills a collapsed editor; the drawer is flat; #1093 covers keyboard, phone layout and axe, and its residue is listed when it lands | #1093, then #1–2 |
| 12 | **A board screenshot in the README** (thread 12) | S | A cropped board and Live bar image in `docs/assets/`, captured by a Playwright script against `ui_test_server` so it can be retaken, in the README beside the recording | No UI image exists since #1071 removed them; the condition it set (handles and thread state, #1082) is met | #1–2 |
| 13 | **A mutating-route classification contract** | S–M | A contract lists every HTTP operation as reading or changing state and fails when a new route is unclassified, or when a `GET` writes an attributed record | Only MCP has `READ_ONLY_TOOLS`; the guarantee that every change is recorded (411.11) is enforced by the request layer but not listed | — |
| 14 | **Split `static/index.html` into modules** (thread 27) | M | CSS and ES modules per surface (board, threads, drawer, session), no build step; `ui_js_contract` and the Playwright specs pass unchanged | One 4,789-line file that every UI change lands in | #1, #2, #11 |
| 15 | **SDK 0.2: typed results and a typed error taxonomy** | M–L | The four SDKs return typed DTOs for the documented operations and map RFC 9457 `type` URIs to error types | The SDKs return generic JSON; 0.2 added retries and paging only (#1096) | — |
| 16 | **MCP stateless subscription delivery across replicas** | M | A stateless subscription is delivered by whichever replica holds the stream, over the existing NOTIFY path | A stateless subscription stays in the replica that took it and fails closed elsewhere (Decisions) | — |
| 17 | **Verification depth** | M each | Fuzz targets over the MCP JSON-RPC and A2A envelope decoders, beside the five in `fuzz/`; `cargo-mutants` over `maidan-auth` and `maidan-bus` nightly; Kani proofs of capability containment and cursor arithmetic | `fuzz/` covers the hand-written parsers only (#1111); `nightly.yml` mutates the store and artifacts only; no Kani | — |
| 18 | **Supply chain** | S–M each | An SBOM attestation (`cosign attest`) on every image; `osv-scanner` over the four SDK lockfiles and `fuzz/Cargo.lock` in CI; `cargo-vet` | `release.yml` signs the SBOM as a blob, with no `attest`; no osv step; those lockfiles are outside `cargo-deny` | — |
| 19 | **Backups and the append-only tables** | S, then L | A tested SQLite backup runbook (`VACUUM INTO`), then per-table autovacuum and range partitioning of the event log, audit and delivery tables, with retention as `DROP PARTITION` | Production and Operations document no SQLite backup path; the tables are unpartitioned | — |
| 20 | **Small event and long-poll gaps** | S each | `MemberFrozen` and freeze expiry; `wait_for_claim_failed`; `wait_for_blocked_resolved` | None exist in `crates/`, so waiters poll for these states | — |
| 21 | **Projector depth** | M, S, S | GitHub App JWT exchange and Check Runs; a `thread_id` index on Slack channel links; a signal when a linked PR closes unmerged | `github.rs` takes a PAT or installation token only; `0051_slack_channel_links.sql` indexes `workspace_id` only | — |
| 22 | **Secret substitution on every egress path** | M | `secret://` references are substituted on A2A push and automation egress, as on webhooks, against a per-workspace allowlist | The broker runs only in `webhook_worker.rs`, and its allowlist is one environment variable for the instance | — |
| 23 | **SCIM groups and renames; browser web push** | M each | SCIM `Groups` and `userName` rename; the `/ui` registers a push subscription; push delivery is retried durably | No Groups routes; the `/ui` has no subscribe flow; a failed push is not retried | — |
| 24 | **A2A `ListTasks` access pushdown** | M | `ListTasks` filters by thread access in the query, not after it | `a2a_agent/ops.rs` fetches batches and filters afterwards, which costs only at scale | — |
| 25 | **Adoption** | S–M each | An OpenHands claimant recipe; provider recipes I2–I6 ([Providers](Providers.md)); the official third-party SDKs run as clients in CI | Retro 418's carry-forward | — |

**Measure before scheduling:** the per-response query count (F-51); whether
the workspace context pack filters before it builds; the search deny-set
query; the board's full re-render on large channels (thread 17); `catch_up`
page cost as the event log grows (thread 18).

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
- **U — the web UI:** design tokens; a code editor, log stream and DAG view; a reference-graph pane over the backlink index; an interactive OpenAPI reference; recorded demos; one-click token mint in Connect; a member last-active endpoint; cross-channel palette search; delivery and allowlist panels.
- **Federation** (when federation gets real multi-peer use): a per-peer egress allowlist in place of the whole-guard `MAIDAN_ALLOW_PRIVATE_EGRESS` bypass, which production already refuses (thread 10); a versioned envelope contract; cross-peer causal order; verifiable-credential peer trust.
- **Parked evaluations** (revisit on a concrete trigger): OPA/ABAC; Tantivy; compile-time SQL; SOPS; a CI egress allowlist; an in-flight-run drain signal; allocator and zero-copy micro-evaluations; an MCP registry policy.
- **L — launch** (the maintainer's call): positioning, the public site and funnel, the in-browser playground, the trust page, comparison and migration pages, and the open-core boundary (the room stays open source; the hosted control plane is the paid tier).

## Decisions pending the maintainer

| Question | Options | Recommendation | Blocks |
|---|---|---|---|
| Require branches to be up to date before merging (F-43)? | Turn on `strict`; or keep it off | Turn it on. Two green PRs broke `main` together on 2026-09-29: each bumped the route-count pin from the same base | Nothing, but every stale-base merge is a risk |
| Promote non-required checks to required? | `a2a tck`, `mcp inspector`, `loom`, `tla`, `pitr drill`, `coverage (llvm-cov)` | Promote `a2a tck` and `mcp inspector` once each is green for two weeks; keep `coverage` advisory. Until then, the drift policy in [Conventions](Conventions.md) applies | Their regressions going unnoticed |
| Should a browser session get `thread:transition`? | Yes, so a person can start review and close from the UI; or keep it token-only | Yes, under the same separation-of-duties checks | The UI's review actions |
| When to publish SDK 0.2 (`sdk-*` tags)? | Now, with retries and paging (#1096); or after typed results (Next #15) | Now, as 0.2.0; typed results as 0.3 | SDK users getting retries |
| How long do read notifications and the usage ledger live? | A retention knob for each; or keep forever | Read notifications: `MAIDAN_RETENTION_NOTIFICATIONS_DAYS`, off by default. Usage ledger: keep, since it is a billing record | Unbounded growth of `maidan_notifications` |
| Squash commits: keep the commit messages, or use the PR body? | `COMMIT_MESSAGES` (today); `PR_BODY` | `PR_BODY`, so each squash commit carries its retro, as Decisions says it should | Nothing |
| Public launch, a hosted playground, a paid tier | — | — | Program L |

External setup that blocks rows, and is not a decision: live Slack and GitHub
apps for projector tests, a provider key for the Goose recipe, and Jev access
for J-01.

## Recently decided

Each links its record. Entries roll off after about a month.

| Date | Decision | Record |
|---|---|---|
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
| 2026-09-23 | Delegated refusals are stored; anonymous ones are counted | [Claims](Claims.md) |
| 2026-09-23 | F-54: no `/v1` and no compatibility promise before 1.0 | [Decisions](Decisions.md) |
| 2026-09-23 | F-48: one delegation list for both backends; Tier 3 declined | #1100 |
| 2026-09-16 | `Maidan-Room-LSN` is the caller's room head (398.8), on outbound paths too since 2026-09-29 | #893, #1105 |

## Known risks

| Risk | Mitigation | Residual | Watched by |
|---|---|---|---|
| A pasted bearer token lives in `localStorage` | Tokens are capability-scoped and revocable; #1093 escapes the `/ui`'s remaining unescaped `innerHTML` writes | Any script injected into the page can read the token | Next #3 |
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
| Wildcard egress selectors; MCP twins of the egress allowlist | An allowlist must name ids; the allowlist is operator policy | [Result Delivery](Result%20Delivery.md) |
| J3: `ttlMs`, `cacheScope`, `server/discover`, `request_client` | Optional MCP `2026-07-28` features no client relies on | [Retro 303](https://github.com/david-engelmann/maidan/blob/main/docs/Retros/Cluster%20303.0.md) |
| Retroactively tagging `v411.0.0` | It would rewrite a recorded history; the record says "never tagged" | Thread 22, #1078 |

## Enhancement thread 2026-09-29: dispositions

David's auditor re-verified each item five times against `main` and freshened
it to `70a6d78b`. Each item's disposition, and where it lives now.
Items 5, 21 and 25 were struck by the thread itself (false premises) and are
not re-filed.

| Thread | Item | Disposition | Where |
|---|---|---|---|
| 1 | Inline errors instead of `alert()` | **Adopt**, P0 | Next #1 |
| 2a | Warn that the token is stored; surface rotation | **Adopt**, P0 | Next #3 |
| 2b | httpOnly cookie sessions for the UI | **Adapt**: the session cookie already exists and is `HttpOnly; SameSite=Lax` (`session/mod.rs`). The work is exchanging a pasted bearer for it, and verifying or removing the unread `csrf_secret` | Next #3 |
| 3 | One primary thread surface | **Adopt**, P0 | Next #2 |
| 4 | Inline attachment previews | **Adopt**, with `nosniff` and a sandboxing CSP on artifact bytes | Next #8 |
| 6 | Header cleanup | **Adapt**: the residual after #1086's Connect sheet | Next #7 |
| 7 | Edit affordance on messages | **Adopt** | Next #11 |
| 8 | Reorganize the tools drawer | **Adapt**: the palette (#1086) is the fast path; group the tabs and drop the Work tab's copy of the board | Next #11 |
| 9 | Dead `#live-*` CSS | **Adopt**, with item 1 (same file, same PR) | Next #1 |
| 10 | Per-peer egress allowlist | **Defer** until a real federation peer; production already refuses the bypass | Later, Federation |
| 11 | Token rotation endpoint | **Adopt**, with 2a | Next #3 |
| 12 | Board screenshot in the README | **Adopt**, after the P0s so the image shows the fixed UI, captured by a script | Next #12 |
| 13 | The recording, diagram and session in the docs site | **Adopt** | #1113 |
| 14a | Cluster 413 overstates the D-A guarantee | **Adopt** | #1113 |
| 14b | Make the D-A guarantee tighter | **Adapt**: widen `authority_audit_contract` to every crate with the two carve-outs named; pinning, not a hole | Later, S |
| 15 | gRPC TLS by default | **Adapt**: refuse plaintext off loopback without an acknowledgement now; native TLS later | #1114; Later, R |
| 16 | A2A push accepts `http` | **Adopt** | #1114 |
| 17 | Board render for large lists | **Defer**: measure first | Measure before scheduling |
| 18 | `catch_up` pagination at scale | **Defer**: it is keyset-paginated with a limit, so flat by construction; measure before changing | Measure before scheduling |
| 19 | README says 165 tools | **Adopt**, with a contract test | #1113 |
| 20 | Conformance jobs required, or a drift policy | **Adapt**: the drift policy is written (Conventions); promotion is the maintainer's | #1113; Decisions pending |
| 22 | Release checklist step for tagging | **Adapt**: tag the day the record merges or relabel it never-tagged; no retroactive tag | #1113 |
| 23 | Record the TCK run summary | **Adopt** | #1113 |
| 24 | First-run onboarding | **Adapt**: the residual after #1086 | Next #7 |
| 26 | Narrow-viewport and keyboard QA | **Adapt**: verify #1093 when it lands and fill only its gaps | Next #11 |
| 27 | Split `static/index.html` | **Adopt**, after the P0s | Next #14 |

**Sequencing.** The three P0s (Next #1–3) come first. Items 1 and 3 of the
thread rewrite the same `static/index.html` that #1086 and #1093 are
rewriting, so they start the moment #1093 lands; the server half of Next #3
(the rotation endpoint) does not touch the page and starts now.

**Where the thread was wrong.** Item 2b assumed the UI had no cookie session:
`crates/maidan-server/src/session/mod.rs` already issues `HttpOnly;
SameSite=Lax` (and `Secure` when configured) cookies for OIDC sign-in, and each
session stores a `csrf_secret` that no handler reads, so `SameSite=Lax` is the
CSRF defence in force. Everything else checked out.
