# Cluster 380 retro — inline per-finding PR review comments

Cluster 379 delivers a waiter result as one GitHub issue comment (`rendered`)
or one Slack message (`summary`). This cluster is the producer's next ask: each
usable finding becomes an inline pull-request review comment on the diff, at
the lines the producer named, on the commit the producer named.

Three impl PRs (380.1–380.3) + this retro. The Cluster 379 summary path is
**unchanged**. Maidan does not approve or request-changes; `event` is always
`COMMENT`.

**Cluster 381 is not unparked by this retro.** It is already open (the
`result_kind` namespaced-string search facet, remaining half of Open Work
row #24). Cluster 382 (the pack half of that row) is already closed.

## What shipped

- **380.1 (#803) — the frame.** `findings[].line_range` is a 1-indexed
  inclusive span on the **post-image** file at envelope `head_sha` (the file
  as that commit left it, not a diff-hunk relative). On GitHub that is the
  **RIGHT** side. `WaiterResult.head_sha` / `findings` parse; `github_line()`
  is `end`; `github_start_line()` is `Some(start)` iff `start != end` (omit
  on a single-line finding or GitHub 422s). `review_commit_id()` is **only**
  envelope `head_sha` — there is no live-PR-head helper. Fixture
  `crates/maidan-types/tests/fixtures/waiter_result_v1.json` carries
  `head_sha` `b5e54f94fd04d6ef7d6e1197ddd59ace70edb911` and two findings on
  `auth.py`.
- **380.2 (#805) — the review POST.** After a successful 379 GitHub summary
  comment, `GithubSender::create_review` posts
  `POST /repos/{repo}/pulls/{n}/reviews` with `commit_id = head_sha`,
  `event: COMMENT`, `side`/`start_side` **RIGHT**, and at most 100 comments
  (GitHub's cap; extra findings dropped, not split). Finding bodies are
  mention-defused (`github_review_comment_body`); they do **not** carry the
  379 recovery marker. A missing sha, empty findings, a non-`reviewed`
  status, or Slack skips the review without sinking the summary. A review
  error never fails the outbox (that would duplicate the issue comment on a
  first delivery). Metric `maidan_github_review_total{outcome}`.
- **380.3 (#806) — skip vs fail, and the remaining e2e.** 404/422 meter
  `skipped` (will not recover on replay). 5xx / rate-limited 403 / 401/403
  meter `failed` so operator replay retries the review (PATCH summary + a
  new COMMENT review on the current `head_sha`). Review errors never
  `disable_link`. Dual-surface envelopes post both summaries and review only
  on GitHub. A vanished envelope at send time still posts the outbox
  snapshot and skips the review. Projector rows never call `create_review`.
- **380.4 — this retro + the doc-close.**

## Decisions

- **`commit_id` is envelope `head_sha` only.** Resolving the live PR head at
  delivery time would pin comments to a commit the producer did not review.
  A missing or unusable sha skips the review; it does not guess. Documented
  on `WaiterResult::review_commit_id` so the next reader does not "helpfully"
  add a `GET /repos/{repo}/pulls/{n}` lookup.
- **RIGHT, never LEFT.** A finding quotes a line in the resulting file. LEFT
  is deletions that no longer exist in the after-state. `start_line` is
  omitted when `start == end` because GitHub 422s that pair.
- **`event: COMMENT`.** Maidan delivers findings; it does not speak a verdict
  on the producer's behalf. APPROVE / REQUEST_CHANGES would be a product
  decision this cluster did not take.
- **The review never fails the 379 outbox.** The summary has already posted.
  Retrying that outbox row on a first delivery would duplicate the issue
  comment (`external_ref` is not yet stored). Replay is the recovery path
  for a transient review failure.
- **404/422 skip; 5xx/auth fail.** The product distinction is
  replay-recoverability, not "did GitHub answer". A 422 on a line that is
  not in the diff at `head_sha` will 422 again. A 5xx will not.
- **Same-SHA replay stacks a COMMENT review.** There is no last-posted-sha
  column. The 379 summary is the in-place object; inline comments are
  additive. Splitting >100 findings across reviews was declined (a second
  review would look like a second verdict).

## Surprises

- **`clippy::await_holding_lock` is lexical, not `drop(guard)`.** The mutex
  guard's lifetime must end at a block before `.await`. Dropping it in place
  is not enough.
- **A stacked PR targeting a deleted base is closed, not retargeted.** Same
  lesson as 379.4 / #790 and 382.2 / #795. 380.2 (#805) and 380.3 (#806)
  targeted `main` from the start and rebased onto the parent squash before
  the parent branch disappeared.
- **Playwright "failure" on the 380.2 rebase was runner acquisition.** The
  job never started (`failed to be acquired (5 attempts)`). 380.2 does not
  touch `/ui` or `ui-tests`. An empty commit retriggered CI.
- **Mock `create_review` records then fails.** A 5xx e2e can assert the
  worker *attempted* the payload GitHub rejected, then replay a second POST.

## Test evidence

- Types: fixture lock + `github_line` / `github_start_line` / RIGHT;
  `review_commit_id` is envelope-only (no live-head helper).
- Worker unit: `prepare_inline_review` (reviewed + sha + findings → payload;
  missing sha / unusable findings / non-reviewed → `None`; mention defuse;
  100-comment cap).
- Server: `result_delivery_inline_e2e` — fixture RIGHT/`head_sha`; no sha;
  bad findings; mention defuse; Slack-only; non-reviewed; 422; 404; re-review
  on a new sha; GitHub+Slack together; 5xx + replay; 403 and rate-limited 403
  vs `disable_link`; vanished envelope; projector kind-split.
- Wire: `egress_wire_e2e` — `POST /repos/{repo}/pulls/{n}/reviews` with
  `event: COMMENT`, RIGHT, omit `start_line` on single-line; 422 →
  `is_unprocessable`; 404 → `is_not_found`; 500 and rate-limited 403 are not
  misconfigurations and not inline-skips.
- `mdbook build` with the linkcheck renderer (`docs/Result Delivery.md` is
  published).

## Forward look

**Cluster 380 is complete.** A `reviewed` GitHub delivery now posts the 379
summary comment **and**, when `head_sha` and usable findings are present, a
COMMENT review of those findings on the post-image RIGHT side of that
commit.

Deferred (follow-ups): a last-posted-sha column so a same-SHA replay would
no-op the review; splitting >100 findings (declined); `/ui` deliveries
panel (379 deferral); recovering a lost Slack `ts` without re-posting (379
deferral).

**Not unparked:** Cluster 381 (`result_kind` facet) is already open. Wave 2
#25 (LandGate) stays the next unstruck *product* row; this retro does not
reorder it. Cluster 382 is already closed.

## Acknowledgements

Three impl PRs (#803 the frame → #805 the review POST → #806 skip/fail/replay
e2e) + this retro. 380.1 was on `main` before 380.2 opened. 380.2 (#805)
rebased onto the Cluster 382 retro squash when that PR landed under it.
