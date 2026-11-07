# Result delivery: the external last mile

**Audience:** you write an agent (a *result producer*) that sets a structured
result on a Maidan thread, and you want that result to reach a surface outside
Maidan: a GitHub pull request, a Slack thread, a Linear issue. This page is the
contract between your producer and Maidan. Read it alongside
[Integrating with Maidan](Integration.md).

## How delivery works

1. Your producer writes a `maidan.waiter.result/1` envelope with
   `set_thread_result` (MCP) or `PUT /threads/:id/result` (REST). The envelope
   names where it should go in `deliver_to`.
2. An operator has allowed each destination once, on the workspace's egress
   allowlist ([the trust model](#the-trust-model--deliver_to-selects-the-workspace-allowlist-authorizes)).
3. Maidan delivers the result to every named target that is on the allowlist,
   through a durable queue that retries, dead-letters and can be replayed.
   A later result on the same thread edits what it delivered, in place.
4. You confirm where it landed with the [delivery status](#delivery-status)
   API.

A correct `deliver_to` can still deliver nowhere when its target is not on the
allowlist. That is a normal outcome, not a producer bug.

What a result can do on each surface:

- **GitHub:** a summary comment on the pull request or issue, then inline
  review comments for each finding ([Inline findings](#inline-findings)).
- **GitHub branch:** a commit of the producer's diff and a draft pull request
  ([The change flow](#the-change-flow-pichangeresult1--github_branch)).
- **Slack:** a message, or a reply in a thread.
- **Linear:** a comment on an issue, with an optional state move
  ([Linear](#linear)).

The grammar is frozen at `maidan.waiter.result/1`. Adding fields is free; a
change to the meaning of an existing field, or to the `deliver_to` shape, needs
a new `schema` value ([Versioning](#versioning)).

---

## What Maidan reads from the envelope

Maidan is a **tolerant reader**. The result is opaque JSON owned by the producer;
Maidan parses only the fields it routes on and ignores everything else. Adding a
field never breaks delivery.

| Field | Required | How Maidan uses it |
|---|---|---|
| `schema` | yes | Envelope discriminator. Must be `maidan.waiter.result/1`. An unrecognized value means no delivery is attempted. |
| `result_kind` | yes | Which producer shape this is, e.g. `example.review.result/1`. Recorded, and filterable ([Discoverability](#discoverability)). |
| `status` | yes | Delivery happens only on `reviewed`. Any other value delivers a short **Maidan-authored** failure notice instead — never silence, never a clean pass. |
| `deliver_to` | no | The routing list. Absent or empty is **valid and normal**: thread-only, delivered nowhere. |
| `rendered` | on `reviewed` | The delivery body. Producer-authored trusted markdown. |
| `summary` | on `reviewed` | One line. The Slack body and the notification title. |
| `view_url` | no | A backlink appended to every delivery. |
| `pr` | no | A human back-reference echoed into the delivered body. |
| `head_sha` | for inline comments | GitHub `commit_id`. 40- or 64-char hex. Absent or unusable ⇒ no inline review (the summary comment still posts). **Never resolved from the live PR head.** |
| `findings` | no | Read twice. For [inline comments](#inline-findings), each usable finding needs `file`, `body` and `line_range`. For the [close-gate](#close-gate), any finding with `severity` exactly `critical` on a reviewed `example.review.result/1` from a producer holding the `review` skill is a `request_changes`; a critical finding without `file`, `body` or `line_range` still counts. Other finding fields stay in the stored JSON, unread. |

`run_id` is **not** a delivery-routing field. Maidan keeps it as the thread's
`parent_run_id` ([Run lineage](#run-lineage)). Every other field in the
envelope (for example `cost_usd`, `duration_secs` or `finding_count`) is
stored untouched and not interpreted.

**The `Finding` shape is the producer's.** Maidan stores the envelope byte for
byte, then *projects* `file`, `line_range` and `body` for the GitHub review,
reads `severity` for the [close-gate](#close-gate) only, and reads `run_id` for
lineage only.

**Cost is not read from the result.** Report a run's tokens, cost and turns
with `report_usage` (MCP) or `POST /threads/:id/usage` while you hold the
claim. Wall time is measured by Maidan from the claim's acknowledgement, not
reported. A `cost_usd` or `duration_secs` in the envelope is stored and never
counted ([Budgets and usage](Integration%20Reference.md#budgets-and-usage)).

---

## Run lineage

A producer's envelope may carry a `run_id`: the id of the run that produced
the result, in the producer's own system. Maidan keeps the producer's value as
the thread's `parent_run_id`; it does not mint a parallel id. Threads that
share a `parent_run_id` can then be listed and counted together.

Maidan reads `run_id` from any result, whatever its `schema`. The value is
trimmed; an empty value, or one longer than 256 bytes, is ignored. It never
changes where the result is delivered.

`set_thread_result` (REST `PUT /threads/:id/result` and the MCP tool) stores
the value as `parent_run_id` when present. This is best effort: a lineage
failure never undoes a stored result. A caller can also set the lineage
directly.

| Surface | Capability |
|---|---|
| `PUT` / `GET` / `DELETE /threads/:id/lineage` | write/delete = `thread:transition`; get = `workspace:read` |
| `GET /workspaces/:id/run-threads?parent_run_id=` | `workspace:read` (private-channel rows the caller cannot access are dropped) |
| `GET /workspaces/:id/run-occupancy?parent_run_id=` | `workspace:read` (empty / unknown run → zeros, not 404) |
| MCP `set_thread_lineage` / `get_thread_lineage` / `list_run_threads` / `get_run_occupancy` | the twins of those routes |

Run occupancy counts every **open** thread in the workspace that shares the
`parent_run_id`, split into `queued`, `claimed`, `working` and `blocked`.
Muting a thread does not change this: a muted nested thread still counts.

To follow one member's occupancy, or read the manager digest, see
[Integration reference: Claiming](Integration%20Reference.md#claiming).

---

## Inline findings

**Frame of reference.** `findings[].line_range` is a 1-indexed
**inclusive** span (`start`..=`end`) on the **post-image** file at `head_sha` —
the file as that commit left it, not a diff-hunk relative offset. On GitHub's
split view this is **RIGHT**. `LEFT` is deletions that no longer exist in the
after-state; a finding that quotes a line in the resulting file is never LEFT.

GitHub mapping for `POST /repos/{repo}/pulls/{n}/reviews` `comments[]`:

| Envelope | GitHub |
|---|---|
| `file` | `path` |
| `line_range.end` | `line` (last line of the range) |
| `line_range.start` when `start != end` | `start_line` (omitted for a single-line finding) |
| (always) | `side: "RIGHT"` |
| `body` | `body` (producer text as written) |
| `head_sha` | `commit_id` |

`event` is `COMMENT`. Maidan delivers findings; it does not approve or
request-changes on the producer's behalf. The worker posts the review after a
successful GitHub summary comment. A missing `head_sha`, no
usable findings, a non-`reviewed` status, Slack-only `deliver_to`, a vanished
envelope at send time, or a GitHub **404/422** skips the review and still
delivers the summary (`maidan_github_review_total{skipped}`). A GitHub **5xx**,
rate-limited **403**, or revoked-token **401/403** also leaves the summary
delivered (`{failed}`) — failing the outbox after the comment has posted would
duplicate it on retry. Operator **replay** PATCHes the summary and POSTs another
COMMENT review on the current envelope `head_sha` (the recovery path for a
transient review failure). Review errors **never** disable a projector
issue-link. Dual-surface envelopes (GitHub + Slack) post both summaries; only
GitHub creates a review. Projector rows aimed at the same issue never call
`create_review`.

Each result POSTs a new COMMENT review (a re-review on a new `head_sha` lands
on that commit); the summary comment is the object that updates in place.

---

## The `deliver_to` grammar

An array of target objects. Each names a `surface` plus per-surface detail:

```json
[
  { "surface": "github", "repo": "example/repo", "pr": 3915 },
  { "surface": "slack",  "channel": "C0123ABCDEF" },
  { "surface": "slack",  "channel": "C0123ABCDEF", "thread_ts": "1759500000.000100" },
  { "surface": "github_branch", "repo": "example/repo", "branch": "feature/agent-fix-1a2b", "base": "dev" },
  { "surface": "linear", "team": "6a1f1c4e-2f0b-4d5e-9a7b-0c1d2e3f4a5b", "issue": "ENG-123", "state": "In Review" }
]
```

### Rules

1. **`surface` is a lowercase identifier.** Known values today: `github`,
   `slack`, `github_branch`, `linear`.
2. **An unknown `surface` is skipped with a recorded warning, never an error.**
   Partial delivery is the model: one target failing or being unroutable must not
   sink the others.
3. **`github` requires `repo` (`owner/name`) and `pr` (an integer issue or PR
   number).** Delivery is a PR/issue comment.
4. **`slack` requires `channel`, and it MUST be a channel ID** (`C…`/`G…`), not a
   `#name`. Names are mutable and ambiguous, and an allowlist keyed on a mutable
   name is not an allowlist. An optional `thread_ts` (a Slack message
   timestamp, `1759500000.000100`) posts the delivery as a reply in that
   thread; the allowlist still blesses the channel. A `thread_ts` that is not a
   Slack timestamp makes the target unusable (a recorded skip), rather than
   posting top-level.
5. **`github_branch` requires `repo` (`owner/name`), `branch` and `base`.** It
   commits a [`pi.change.result/1`](#the-change-flow-pichangeresult1--github_branch)
   to `branch` and opens a draft pull request against `base`. It is logged as
   `github_branch:<repo>@<branch>`. `branch` must match
   `feature/agent-[a-z0-9][a-z0-9-]*`; `base` must be blessed for the
   repository, and is never `prod`.
6. **`linear` requires `team` (the team's id, a UUID) and `issue` (its
   identifier, `ENG-123`, or its id).** Delivery is a comment on the issue.
   An optional `state` names a workflow state of that team, by its exact name,
   to move the issue to once the comment lands. A `team` that is not an id, or
   a blank `state`, makes the target unusable (a recorded skip). See
   [Linear](#linear).
7. **An empty or absent list delivers nowhere.** This is a valid, supported
   outcome — not a misconfiguration.
8. **New surfaces add new object shapes.** Rule 2 makes that backward-compatible:
   an older Maidan skips a surface it does not know.

### Where the producer gets it

The task author sets the routing intent in the seeding message's metadata under
`deliver_to`; the producer carries it through opaquely and echoes it into the
result envelope. Maidan reads it from the **result envelope**, not from message
metadata.

---

## The trust model — `deliver_to` selects, the workspace allowlist authorizes

**This is the part a producer must understand, because it changes what "success"
means.**

A result is written by an agent. Any caller holding `thread:transition` on the
thread can write any `deliver_to`. Maidan's connector credentials, by contrast,
are operator-held and can reach many repositories and channels. Routing straight
off an agent-supplied list would make Maidan a confused deputy: a compromised or
merely buggy agent could post under Maidan's identity anywhere that credential
reaches.

So delivery is authorized twice:

- **`deliver_to` selects** which of the permitted targets this particular result
  goes to. The producer stays in control of routing and no surface is hardcoded.
- **A per-workspace egress allowlist authorizes.** An operator blesses
  `github:example/repo` or `slack:C0123ABCDEF` once, over
  `POST /workspaces/:wid/egress-targets` (`token:admin`; `GET` lists,
  `DELETE …/:tid` revokes). A target absent from the allowlist is **skipped with
  a recorded warning** — the same treatment as an unknown surface.
  - A selector must be an **id**, never a name: a Slack channel id (`C…`/`G…`),
    or a GitHub repository `owner/name`. A `#channel-name` is refused, because
    the channel a name points at can change under the blessing.
  - On GitHub the blessing is the **repository**, not the issue — one blessing
    covers every PR in it, so an operator does not file a ticket per review.
- **The allowlist defaults to empty**, following the same fail-safe as Maidan's
  secret-egress broker: with nothing configured, nothing is trusted and nothing
  is delivered.

### Operator setup — blessing a target

One `token:admin` call per destination, per workspace:

```bash
curl -sS -X POST "$MAIDAN/workspaces/$WORKSPACE_ID/egress-targets" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  -d '{"surface":"github","selector":"example/repo"}'

curl -sS -X POST "$MAIDAN/workspaces/$WORKSPACE_ID/egress-targets" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  -d '{"surface":"slack","selector":"C0123ABCDEF"}'
```

For the change flow, bless each repository and its base under
`github_branch`, as `owner/name@base` (its own rows; a `github` comment
blessing does not allow commits), and the Slack channel the replies go to. A full seed, as the audited calls, is in
[Production](Production.md#result-delivery-to-github-and-slack).

`GET` the same path lists what is blessed; `DELETE …/egress-targets/:tid` revokes.
Blessing is idempotent — a re-bless returns the existing entry with its original
`created_at`, so the list stays "what may we post to" with nothing to reconcile.

**The selector is two fields, not one string.** There is no `github:owner/repo`
target syntax at this boundary: `surface` is one of `github`, `slack`,
`github_branch` or `linear`, and `selector` is the per-surface id. The rules,
enforced on every write path (`400` with the reason on a violation):

| Surface | `selector` | Rejected |
|---|---|---|
| `github` | a repository, `owner/name` | anything containing `#` — the blessing is the **repo**, not the issue, so one call covers every PR in it |
| `github_branch` | a repository and the one base change pull requests may target, `owner/name@base` (one row per base) | a selector without a base, a `prod` base, an agent branch as a base. A separate surface with its own rows: a repository blessed for `github` comments is **not** writable by the change flow |
| `slack` | a channel id, `C…` or `G…` | `#channel-name` — a name is mutable, and the channel a name points at can change under the blessing |
| `linear` | a team id, a UUID | a team key (`ENG`) or name — a key can be renamed, and another team can take the old one |

Leading/trailing whitespace is refused on both. The authorization key is coarser
than the delivery target on GitHub: a result aimed at `example/repo#42` is
authorized by the `example/repo` blessing.

### What this means for the producer

**A perfectly correct `deliver_to` can still deliver nowhere.** That is not a
producer bug and must not be reported as a failure. Treat "delivered nowhere" as
a normal outcome; the per-target disposition is readable from the delivery-status
API below, and an operator — not the agent — fixes an unblessed target.

---

## The change flow (`pi.change.result/1` → `github_branch`)

`pi.change.result/1` is the change-result schema: the `result_kind` of a
result that carries a diff for Maidan to commit.

A coding agent edits a checkout but holds no GitHub write credential. It
returns the commit it started from and a diff. Maidan, holding the operator's
token, makes the commit, opens a **draft** pull request, and answers in the
Slack thread the work started in.

**The envelope.** `result_kind` is `pi.change.result/1`. Besides the usual
envelope fields, Maidan reads:

| Field | How Maidan uses it |
|---|---|
| `status` | `changed`, `no_change`, `seat_error`, `content_blocked` (the producer found a secret in the diff, title or summary, and sent none of it) or `rebase_conflict` (a squash-rebase round whose rebase stopped on conflicts in the producer's checkout). Only `changed` writes to GitHub. Any other status records the `github_branch` target as skipped, with the status in the reason, makes no GitHub call, and the Slack reply says the status; for `rebase_conflict` it says the rebase stopped on conflicts and gives `error_detail`, where the producer lists the conflicting paths. |
| `rebase` | Optional. `true` (a JSON boolean; anything else is a normal round) makes this a **squash-rebase round**: `base_sha` is a commit of the target's base branch (its head when the round started) and `diff` is the whole branch's change against it. See below. |
| `branch_head_sha` | Required for a squash-rebase round, ignored otherwise: the change branch's remote head when the round started, as a full SHA (40 or 64 hex, any case). The force move is leased on it. Absent or not a full SHA, a rebase round is refused. |
| `base_sha` | The full SHA (40 or 64 hex) the diff applies to, captured before any edit. The commit's parent. **Never** taken from thread metadata or the branch. |
| `branch` | The branch the producer worked on, echoed verbatim. Must equal the target's `branch`, or the delivery is refused. |
| `diff` | A git diff that applies with `git apply` to `base_sha`. |
| `error_detail` | Optional. Why a run that changed nothing ended. The Slack reply to a status other than `changed` gives it after the status, in place of `summary`, with a cause repeated across retries said once and the whole cut at 600 characters. Without it the reply gives `summary`. |
| `title`, `summary` | Optional. The commit subject and the draft pull request's title, and the commit and pull request body; mentions in `summary` are defused. Without `title`, Maidan uses the thread's opening message, first line, cut at a word to 72 characters (the branch name if there is none). Without `summary`, the body names the Maidan thread and the commit SHA. Missing text never stops a delivery. |
| `pr_state` | Optional, `{title, body}`. `body` describes the whole branch against its base as it stands after this round (where `title` and `summary` describe this round). Maidan keeps it in one section of the pull request body, between `<!-- pi:current-state -->` and `<!-- /pi:current-state -->`, replacing only that section on each delivery and appending it when the markers are missing, so edits people and review bots make elsewhere in the body stay. Mentions are defused and the section is capped at 20,000 characters. `pr_state.title` is not read: a title people fixed is never reverted. A failed body edit never fails the delivery. |
| `check` | Optional, `{context, state, description}`: the producer's own check of the change before delivery. After the branch is pushed and the pull request opened or found, Maidan posts it as a commit status on the head it delivered (`POST /repos/{repo}/statuses/{sha}`, with the envelope's `view_url` as `target_url` when it is https), so a re-delivery or a rebase round posts it again on the head it leaves. `context` is non-empty, printable and at most 100 characters; `state` is `success`, `failure`, `error` or `pending`; `description` is put on one line and cut to 140 characters. A malformed `check` posts nothing and adds `check_invalid` to the row's `template_warnings`; a status GitHub refuses adds `check_not_posted`. Either way the branch and the pull request stand and the Slack reply says so in one line. A commit status, not a check run, because a token can post one. |

**What a `changed` result does**, per `github_branch` target the workspace has
blessed:

0. Checks the hard-coded rules, at routing and again before any write: the
   branch matches `feature/agent-[a-z0-9][a-z0-9-]*` and is not `prod`,
   `main`, `master`, `staging` or `dev`; the base is not `prod` and not the
   branch; `owner/name@base` is on the workspace allowlist; and no pull request
   for the branch is open into another base. Each refusal is recorded with its
   reason and nothing is written. See
   [Production](Production.md#result-delivery-to-github-and-slack) for the
   full list of what Maidan never does with the token.
1. Reads the branch. When it is missing, it is created at `base_sha` — after
   the diff has been applied in memory, so a diff that does not apply leaves no
   branch behind.
2. Refuses when the branch head is not `base_sha`, or when the result's
   `branch` is not the target's `branch`.
3. Fetches only the files the diff touches, at `base_sha`
   (`GET /repos/{repo}/contents/{path}?ref={base_sha}`), applies the hunks, and
   refuses when one does not match. Supported: modified, added, deleted and
   renamed text files (a rename may arrive as a delete plus an add, which is
   how some producers send it, or as a git rename), `100644`/`100755` modes, `\ No newline at end of file`,
   hunks that moved. Refused: binary patches, copies, symlinks, submodules,
   quoted paths, paths outside the repository or under `.git`.
4. Commits through the Git Data API: one blob per changed file, a tree on the
   base tree, a commit whose only parent is `base_sha`, and a fast-forward of
   the branch (`force: false`). A squash-rebase round is the one exception;
   see below.
5. Opens a **draft** pull request against `base` with the title and summary
   when none is open for the branch. Maidan posts no other comment (except
   a squash-rebase's range, below), requests no review, and never marks the
   pull request ready, approves or merges it. A result with a `check` then
   gets its commit status on the delivered head (see the table above).
6. Replies in the Slack target's thread with the commit SHA and the pull
   request link. The reply waits for the branch delivery; a target that was
   refused or skipped is reported with its reason.

**The template check flags and never refuses.** After each delivered round,
Maidan reads the pull request as it now stands (the title and body it opened
with, or the open pull request's own title and body on a later round, since a
later `title` or `summary` never reaches an open pull request) and records
what it lacks against the producer's template as `template_warnings` on the
`github_branch` delivery row:

- `missing_testing`: no Markdown heading reading `Testing` (any level, any
  case, an optional trailing colon) outside fenced code;
- `missing_issue_key`: no issue key such as `ENG-123` (an uppercase letter,
  uppercase letters or digits, a hyphen and digits, standing alone) in the
  title or the body.

The Slack reply adds one line, such as `PR template: missing a Testing
section.`, and the delivery-status API shows the codes; the field is absent
when nothing is missing. A warning is not a failure: the row is `delivered`,
`last_error` stays empty, and the pull request stays a draft until it is
marked ready, as always. Fixing the pull request itself clears the warnings
on the next round. Apart from defused mentions, `summary` reaches the pull
request byte for byte, headings and fenced check output included, and a
`title` such as `[ENG-123] …` is used as given.

**Refusals are recorded, not retried.** A refused target is `skipped` with the
reason in `last_error` on the delivery-status API (`GET
/threads/:id/deliveries`). A GitHub 5xx or rate limit retries with backoff
like any delivery; a 401/403/404 dead-letters the target.

**Idempotent per thread and result.** The commit message carries a trailer
`Maidan-Change: <thread_id>/<digest>`, the digest covering `base_sha` and the
diff (and, for a squash-rebase round, the `rebase` flag, so a rebase never
takes a normal round's commit for its own). A retried or replayed delivery that finds the branch head is that commit,
on top of `base_sha`, reuses it; the pull request is looked up before one is
opened. A retry makes neither a second commit nor a second pull request.

**A follow-up** is a new result whose `base_sha` is the branch head the last
change left: it commits on top and finds the pull request already open.

**A squash-rebase round** (`"rebase": true`) is a follow-up that rebases onto
the current base instead of merging the base in, for a queue that takes only
linear pull requests. `branch` is the existing agent branch, `base_sha` the
base commit it rebased onto, `branch_head_sha` the branch head the round
started from, and `diff` the whole branch against `base_sha`. On top of
every rule above (the branch shape, the allowlisted base, never `prod`, the
open pull request's base, and the merge-queue refusal below), Maidan refuses
it, before any write and with the reason recorded, when:

- no pull request is open for the branch;
- the result names no `branch_head_sha` ("a rebase result must name
  branch_head_sha, the branch head the round started from");
- **someone else pushed:** the branch head is not the newest round the change
  flow recorded for that pull request in this workspace (a commit another
  workspace delivered counts as someone else's). The reason names both SHAs;
- **the branch moved since this round started:** the branch head is not
  `branch_head_sha`. The reason names both SHAs;
- **the base is not on the base branch:** `base_sha` is neither the base
  branch's head nor one of its ancestors (GitHub's compare of
  `base_sha...<base head>` is not `ahead` or `identical`);
- **the base is older than the branch's:** the merge base of the branch head
  and the base head (the base work the branch already holds) is neither
  `base_sha` nor one of its ancestors, so the move would drop base work the
  branch has. A `base_sha` the base has moved past since the round started is
  accepted when it is no older than that: on a busy base a rebase takes long
  enough that the head rarely stays put. The exact base head passes without a
  compare;
- the diff does not apply to `base_sha`.

Otherwise it applies the diff to the `base_sha` tree, makes one commit whose
only parent is `base_sha` with the usual `Maidan-Change` trailer, re-reads the
branch head immediately before the move, and moves the branch with
`force: true` only if the head is still `branch_head_sha`, the head the round
started from. This is the change
flow's **only non-fast-forward write**. It is allowed only for a
`feature/agent-*` branch whose head is the last commit Maidan delivered, and
only through one guard that no other code path can reach: the force write
takes a single-use pass that only that guard mints, bound to the branch and
both commits, and checks the branch shape again. GitHub's ref API has no compare-and-swap, so a push landing between the
re-read and the move (one round trip) is the window that remains. The round is
recorded with `rebase: true` and how far `base_sha` is behind the base, and the pull request's
current-state section is updated from `pr_state` as for any round. A retry that
finds the head is already that rebase commit (same trailer, parent `base_sha`)
reuses it and writes nothing.

**The rebase's range.** The force move rewrites the history a reviewer has
read, so after it Maidan comments on the pull request
`Rebased <old_head>..<new_head> onto <base_sha>`, and the Slack reply carries
the same line under the commit and pull request link. A reviewer range-diffs
those two heads to see what the rebase changed. The round records the old
head as `previous_head`, which the swarm metrics list with its rounds. This is
the change flow's only comment. It is best effort: a comment GitHub refuses is
logged, and the move, the delivery and the Slack line stand. A retry that
reuses its rebase commit comments only when the move was never recorded, so a
replay of a recorded round writes nothing.

**A rebase conflict** is the producer's to resolve: it sends `status:
"rebase_conflict"`, Maidan records the target as skipped with no GitHub call,
and the Slack reply says the rebase stopped on conflicts and gives
`error_detail` (the conflicting paths, when the producer lists them).

**The merge queue.** For a repository listed in `MAIDAN_MERGE_QUEUE_LABELS`,
the client that marks agent pull requests ready may enter one into the merge queue
by asking Maidan to add the queue's label (`POST /operator/github/enqueue`
with `repo` and `pull_number`), read whether it is labelled (`GET`), and take
the label off (`DELETE`). Enqueue reads the pull request once and refuses
unless every check holds on that read: a label is configured for the
repository; the pull request is open, not a draft and not from a fork; its
head is a `feature/agent-*` branch into the pinned base and on the workspace
allowlist; and its head is the commit the change flow last delivered to it.
The last check keeps a commit pushed after the review out of the queue: the
refusal names both SHAs. Dequeue skips that comparison, since the queue may
have rebased the head, and a pull request already without the label is
answered "was not labelled". Both are recorded in the audit log as
`github.enqueue`.

While a pull request carries its repository's queue label, the change flow
refuses a delivery to its branch before any write: the queue merges the head
it took, and a later commit would land apart from what was reviewed. Dequeue
first, then deliver the follow-up.

**Credentials.** The commit and the pull request are made with
`MAIDAN_GITHUB_TOKEN` (see [Production](Production.md#result-delivery-to-github-and-slack)),
so they are authored by that token's owner. The token needs `contents:write`
and `pull_requests:write` on each repository. With a GitHub App configured
(`MAIDAN_GITHUB_APP_ID`, `MAIDAN_GITHUB_APP_INSTALLATION_ID` and
`MAIDAN_GITHUB_APP_PRIVATE_KEY`), they are made with the app's installation
token instead and authored by the app's bot account.

---

## Per-surface behaviour

### GitHub

- Body is `rendered` verbatim. GitHub renders GFM, so the producer's markdown
  arrives as written.
- A `view_url` backlink is appended.
- Truncated with an explicit marker if it would exceed GitHub's comment ceiling
  (65536 characters). **Do not assume `rendered` arrives whole** — put the
  load-bearing content early and rely on the backlink for the rest.

### Slack

- **Slack does not render GFM.** Slack speaks *mrkdwn*: `*bold*` not `**bold**`,
  `<url|text>` not `[text](url)`, no headings, no tables. Posting `rendered`
  verbatim would ship visibly broken output.
- So Slack receives `summary`, a compact Maidan-projected digest, and the
  `view_url` link — **not** `rendered` as-is. A producer that wants precise
  Slack formatting should make `summary` carry the weight.

### Linear

- The comment body is the same Markdown a GitHub comment gets (`rendered`, the
  PR line, attachments and the `view_url` backlink), cut at 65536 characters.
  A change result instead gets the change reply a Slack thread gets: the
  commit and draft pull request once the `github_branch` delivery lands, or
  the status and the producer's reason when nothing was committed. The
  comment waits for the branch delivery the same way.
- **The issue must be in the team the result names.** The allowlist blessed
  the team, not the issue, so the sender looks the issue up first and refuses
  it (a recorded skip, no write) when it is in another team.
- A later result for the thread **edits the same comment**. A comment deleted
  in Linear since is posted again.
- `state` moves the issue after the comment lands, and every delivery of the
  result moves it again. A state the team does not have, or a move Linear
  refuses, is logged; the comment stands and the delivery is `delivered`.
- The operator sets `MAIDAN_LINEAR_API_KEY` and blesses each team by id:
  `{"surface":"linear","selector":"<team id>"}`.

### All surfaces

- **Mention sequences are neutralized at the egress boundary.** `rendered` is
  producer-authored but quotes attacker-influenced code; an `@org/team` in a
  GitHub comment or a `<!channel>` in Slack would ping real people from
  attacker-controlled bytes. Maidan defuses these. Producers do not need to.

---

## Idempotency

A thread holds exactly one result (`set_thread_result` upserts). Re-running a
review over the same thread therefore **updates the existing delivery in place**
rather than stacking a second comment.

Maidan keys a delivery on `(thread_id, target)` and remembers the external
reference it created — a GitHub `comment_id`, a Slack `ts`. A later result on the
same thread edits that object. A hidden marker at the start of the GitHub comment
body is the recovery path if the stored reference is ever lost.

**Producer obligation:** re-review the *same thread*. A new thread per run is a
new delivery, and will produce a second comment — correctly, since it is a
different unit of work.

---

## Failure semantics

- **Partial delivery is normal.** A connector failure on one target never
  prevents the others.
- **Nothing is silently dropped.** A failed delivery retries with backoff and
  dead-letters after a bounded number of attempts, where an operator can inspect
  and replay it. A misconfiguration-class error (401/403/404) on **projector**
  traffic disables the link rather than retrying forever. The same status on a
  **result** delivery dead-letters that target without disabling the projector
  link — a result must not take down room-to-issue relay.
- **A non-`reviewed` status is surfaced as a failure**, using `status` alone. It
  is never rendered as a passing review and never quietly skipped.
- Every delivery attempt is audited.

## Discoverability

`result_kind` is a **namespaced string**, not a closed enumeration. A producer publishes
`result_kind = "example.review.result/1"` inside `schema = "maidan.waiter.result/1"`; a
future producer ships a new string and the facet works without a server change.

List (not message-FTS) surfaces, `workspace:read`:

- REST: `GET /workspaces/{id}/results?result_kind=example.review.result/1` (`limit`
  optional, default 50, clamp 1–500). Omit `result_kind` to list every
  non-tombstoned result the caller can access.
- MCP: `list_thread_results` with the same optional `result_kind` / `limit`.
  Workspace comes from the token; there is no `workspace_id` argument.

Match is exact on the indexed string. `example.review.result` and
`example.review.result/10` do not hit `example.review.result/1`. Private-channel rows the
caller cannot access are omitted. This is not `GET /workspaces/{id}/search`
and not the ADR JSON convention `"kind": "decision"` in
[Integration reference](Integration%20Reference.md#decision-records).

## Close-gate

A reviewed `example.review.result/1` whose `findings` contain any
`severity == "critical"`, from a producer that holds the `review` skill, is a
`request_changes`. If the thread has no review requirement, Maidan sets one
(`k = 1`: one approval), so the thread cannot close until a person who is
neither its owner nor its assignee approves it.

This uses the existing review gate; it is not a new one:

- A result with only warnings, another `result_kind`, a status other than
  `reviewed`, or a producer without the `review` skill changes nothing.
- An existing requirement is left alone.
- A clean re-review does **not** approve on its own.
- An empty `deliver_to` still sets the requirement: the thread is held even
  when nothing is posted outside Maidan.
- The GitHub review stays `event: COMMENT`. The gate in Maidan decides the
  land; the pull request is not marked `REQUEST_CHANGES`.

Both `PUT /threads/:id/result` and MCP `set_thread_result` set the requirement
when the result is written, and the `ThreadResultSet` event handler sets it
again on every replica and on replay. Both are idempotent.

## Delivery status

Per-thread delivery state is readable over REST and MCP, with a replay action, so
a producer can confirm where its result actually landed without scraping the
external surface.

| Call | Surface | Capability |
|---|---|---|
| `GET /threads/:id/deliveries` | REST | `workspace:read` + thread access |
| `POST /threads/:id/deliveries/:did/replay` | REST | `workspace:write` + thread access |
| `list_result_deliveries {thread_id}` | MCP | `workspace:read` + thread access |
| `replay_result_delivery {thread_id, delivery_id}` | MCP | `workspace:write` + thread access |

One row per `(thread, target)`:

```json
{
  "id": "…", "thread_id": "…",
  "surface": "github", "selector": "example/repo#42",
  "status": "delivered",
  "external_ref": "998877",
  "armed_revision": "2026-09-15T10:00:00Z",
  "delivered_revision": "2026-09-15T10:00:02Z",
  "attempts": 1, "last_error": null,
  "created_at": "…", "updated_at": "…"
}
```

- **`status`** is `pending` (armed, not yet sent), `delivered`, `failed` (the
  transport gave up and the egress queue dead-lettered it, or the server has no
  sender for the surface: `last_error` then names the setting, such as
  `Slack delivery is not configured on this server (MAIDAN_SLACK_BOT_TOKEN)`),
  or `skipped`
  (deliberately not delivered — an unknown surface, or a target the workspace has
  not blessed). `skipped` is **not an error**; `last_error` carries the reason.
- **`attempts`** counts the sends tried, and **`last_error`** holds the latest
  failure. While the egress queue is still retrying, the row stays `pending`
  with both set, so a delivery being retried does not read as one never tried.
- **`selector`** is the full destination (`owner/name#123` on GitHub), which is
  finer than the `owner/name` the allowlist is keyed on.
- **`external_ref`** is the object we created — a Slack `ts`, a GitHub comment
  id. It is what the next revision edits in place.
- **`armed_revision` / `delivered_revision`** are `ThreadResult.produced_at`
  watermarks: newest seen vs newest actually landed. `delivered_revision: null`
  means never delivered. A result is re-delivered only when its `produced_at` is
  newer than `armed_revision` — that is the dedup, and it is why a replayed event
  is a no-op while a genuine re-review is an update.
- **An empty list is `200 []`** — the result was routed nowhere, which is valid.

Replay re-enqueues one row and **re-checks the allowlist**: an unblessed target
stays `skipped` (status is not policy). It does not bump `armed_revision`.

**A target that was not reached is said on the event stream.** Each time a row
moves into `failed` or `skipped`, Maidan appends a `result_delivery_failed`
event in the same transaction, scoped to the thread's workspace, channel and
thread:

```json
{"kind": "result_delivery_failed", "workspace_id": "…", "channel_id": "…",
 "thread_id": "…", "surface": "slack", "selector": "C0123ABCDEF",
 "status": "skipped", "last_error": "target not in the workspace egress allowlist"}
```

It is said once per transition: recording the same outcome again (a replay
against a still-unblessed target) adds nothing, while a new revision of the
result that re-arms the row and is skipped again says it again. A delivered
target says nothing. A relay subscribed to the workspace with
`kinds=result_delivery_failed` (on the WebSocket filter or `GET /mcp/stream`)
can tell the task's own thread where its result did not go.

---

## What Maidan will never deliver

- Raw agent output of any kind: agent stdout, un-rendered model text, tool
  transcripts. **Only `rendered`, `summary`, and structured `findings`.**
- Anything at all when `status` is not `reviewed` (beyond the failure notice).
- Anything to a target the workspace has not blessed.

Maidan does not run agents, does not render reviews, and is not a CI product.
The close-gate reads only `findings[].severity` (`critical` →
`request_changes`); it does not judge finding bodies. A GitHub review's `event`
stays `COMMENT`. Maidan delivers trusted bytes to allowed surfaces, durably,
once.

---

## Versioning

The grammar above is **frozen at `maidan.waiter.result/1`**. Additive fields
are free. A change to the meaning of an existing field, or to the `deliver_to`
shape, requires a new `schema` value.

**An unrecognized `schema` is inert.** Maidan attempts no delivery and records
nothing: no warning, no `skipped` row, nothing on the status API. If a result
is never delivered and the status API is empty, check the `schema` string
first. A reference envelope is
[`crates/maidan-types/tests/fixtures/waiter_result_v1.json`](https://github.com/david-engelmann/maidan/blob/main/crates/maidan-types/tests/fixtures/waiter_result_v1.json).
