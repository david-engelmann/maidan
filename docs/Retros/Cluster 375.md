# Cluster 375 retro — Wave 2 #22: required reviewers (G5 + G-dev-5)

Wave 2 #22 makes a thread's **close** answerable to reviewers: a thread declares
a review requirement (`k` approvals) from a named reviewer set (`n`), a reviewer
submits an approve / request-changes decision, and the FSM refuses `closed` until
`k` distinct **qualifying** approvals exist and no unresolved `refutes` edge
blocks it. **A gate, not a poll/closer** (325) — it doesn't tally votes to decide;
it stands between the thread and `closed`.

## The design (chosen by the maintainer)

A **dedicated review store** (over the "review-child threads" and "reuse votes"
alternatives) — the cleanest domain model: `review_status` is a single counting
query the close-gate reads, and review-child threads can layer on later.

## What shipped

- **375.1 (#759) — the store foundation.** `maidan_thread_review_reqs` (the `k`),
  `maidan_thread_reviewers` (the named `n`), `maidan_thread_reviews` (decisions)
  (pg 0079 / sqlite 0078) + `ReviewDecision`/`ThreadReview`/
  `ThreadReviewRequirement`/`ReviewStatus` + `ReviewStore` (both backends).
  `review_status` counts DISTINCT **qualifying** approvals — decision=approve,
  reviewer ≠ owner/assignee (the Cluster-355 separation of duties), and, when a
  named set exists, in it. Zero-blast-radius.
- **375.2 (#761) — the FSM close-gate.** `review_gate_in_tx` in `transition_in_tx`
  (both backends), gated on `to_state == Closed`: refuses close unless
  `approvals_met` AND no `refutes` reference (`relation=refutes`, `dst=thread`)
  targets the thread → `Conflict`. Additive (a thread with no requirement + no
  refutes closes as before).
- **375.3 (#762) — REST.** `review-requirement` PUT/GET/DELETE, `reviewers`
  POST/GET + DELETE `:member_id`, `reviews` POST/GET, `review-status` GET.
  Governance writes = `thread:transition`; reads = `workspace:read`.
- **375.4 (#763) — MCP.** `set_review_requirement`/`add_reviewer`/`submit_review`
  (thread:transition) + `get_review_status`/`list_reviews` (workspace:read).
- **375.5 — this retro + the doc-close.**

## Decisions

- **The gate lives in `transition_in_tx`, beside the Cluster-355 SoD check.** The
  close-gate is a transition concern; putting it in the shared in-tx transition
  covers REST + MCP + any future transition path in one place, and runs in the
  transition's own tx so it can't be raced.
- **Separation of duties reuses `owner`/`assignee`.** A qualifying approval is
  from neither — the author (owner) and the claimer (assignee) can submit but
  their approval never counts. This is the G5 "author/assignee votes don't count".
- **Named set is the eligible set; empty = open review.** With a named `n`, only
  those members' approvals count; with none, any qualifying member's does — both
  "k-of-n" and open review from one schema.
- **The `refutes` edge is presence-based.** An unresolved `refutes` reference
  targeting the thread blocks close; "accepting" it = removing the reference. A
  richer accepted/withdrawn state on the reference is a noted follow-up.
- **Governance = `thread:transition`.** Setting the requirement, naming reviewers,
  and submitting a review all gate the transition, so they share its cap; reads
  are `workspace:read`. No new capability → no deny-caps-matrix ripple.

## Surprises

- **`required_count` must be `BIGINT` on Postgres.** sqlx maps pg `INTEGER`→`i32`;
  the `i64` model needs `BIGINT` to bind/read cleanly (sqlite `INTEGER` holds i64).
- **The count query binds `thread_id` 4×/3× on sqlite** (the requirement subquery
  + the count filter + the two named-set `EXISTS` subqueries) — positional binds,
  so each `?` is a separate `.bind`.
- **The infra tax:** Docker Hub began denying `minio/minio` **and** `minio/mc`
  pulls mid-cluster ("pull access denied / repository does not exist"), reddening
  `docker compose smoke` + `scale-out smoke` on every PR. Fixed durably by
  repointing both to `quay.io/minio/*` (#760) — a separate chore that unblocked
  the whole pipeline without skipping required checks.

## Test evidence

- Store: `reviews` (SoD exclusion, open vs named set, mind-changing, lists,
  remove, clear) + `review_gate` (0-of-1 blocks close; an approval allows;
  refutes blocks) — both backends. `backend_parity` green.
- Server: `review_rest_e2e` (auth-enabled) + `openapi_e2e` bijection +
  `http_capability_matrix_e2e`.
- MCP: `review_tools_set_require_name_submit_and_status` + both contract-sync
  tests + `mcp_capability_matrix_e2e`.

## Forward look

**Wave 2 #22 is complete.** Deferred (follow-ups): a `refutes`-accepted/withdrawn
state (vs presence-based); review-child threads (a review as its own assigned
thread, the design's alternative — layer on the dedicated store); a
`ReviewSubmitted`/`ReviewSatisfied` event so a waiter reacts without polling
`review-status`; a `/ui` review panel. **Next: Wave 2 #23** (G6 + G-dev-3 + W3 —
a spawn budget).

## Acknowledgements

Four impl PRs (#759 store → #761 gate → #762 REST → #763 MCP) + a CI chore (#760,
minio→quay) + this retro, on the foundation-then-wire + new-route-preflight +
capability-registry patterns.
