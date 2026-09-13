# Result delivery — the external last mile

**Audience:** authors of agents that write a structured result to a Maidan thread
and want it to reach an external surface (a GitHub PR comment, a Slack message).
The first such producer is **pi**, whose code-review waiter publishes a
`pi.waiter.result/1` envelope via `set_thread_result`.

**This page is the interface contract between a result producer and Maidan.**
Read it alongside [Integrating with Maidan](Integration.md).

---

## Status — read this first

**Shipped (Clusters 379–381).** This page is the interface contract between a result producer
and Maidan. The grammar is **frozen** at `pi.waiter.result/1`. Additive fields are
free; a change to the meaning of an existing field, or to the `deliver_to` shape,
requires a new `schema` value. The `result_kind` list facet shipped in Cluster 381.

| Piece | State on `main` today |
|---|---|
| `set_thread_result` / `get_thread_result` / `ThreadResultSet` event | **Shipped** (Clusters 234–236). A result is durable and observable. |
| Slack + GitHub connectors (post, link tables, link management) | **Shipped** (Clusters 307–312, 346, 349.4). |
| Durable retrying delivery queue + DLQ + replay | **Shipped** (Clusters 50, 304–306 for mail and webhooks; **377** for projector egress — a retrying queue, a link that disables itself on a credential failure, and a `token:admin` DLQ at `GET /operator/egress/dead`). |
| The per-workspace egress allowlist (the trust boundary below) | **Shipped** (Cluster 378.1). `POST`/`GET /workspaces/:wid/egress-targets` + `DELETE …/:tid`, `token:admin`. Cluster 379.3 consults it on every target. |
| A `ThreadResultSet` handler that delivers to `deliver_to` | **Shipped** (Cluster 379.3). Fetch → parse → per-target allowlist check then enqueue. |
| Idempotent update-in-place | **Shipped** (Cluster 379.4). Stored `external_ref` → `update_*`; GitHub recovery marker `<!-- maidan:result:<thread_id> -->` at byte 0. |
| Per-thread delivery status + replay | **Shipped** (Cluster 379.5). `GET /threads/:id/deliveries` + `POST …/deliveries/:did/replay` + MCP `list_result_deliveries` / `replay_result_delivery`. |
| Inline per-finding PR review comments | **Shipped** (Cluster 380). After the GitHub summary, a `reviewed` envelope with `head_sha` and usable findings posts `POST /repos/{repo}/pulls/{n}/reviews` (`commit_id = head_sha`, `event: COMMENT`, RIGHT, `line` = `line_range.end`). |
| `result_kind` list facet | **Shipped** (Cluster 381). Exact-match on the namespaced string — see [Discoverability](#discoverability). |
| Run lineage (`parent_run_id`) | **Shipped** (Cluster 387). The producer's `run_id` is accepted as-is and homed on the thread. **Not** a delivery-routing field. See [Run lineage](#run-lineage-cluster-387). |

**Producer loop:** write `deliver_to` on the envelope; bless the destination once over
the allowlist; confirm where it landed with the status API. A perfectly correct
`deliver_to` can still deliver nowhere if the target is unblessed — that is a
normal outcome, not a producer bug.

Inline per-finding PR review comments (Cluster 380) are **shipped**.
**380.1** pinned the `line_range` frame: file-absolute **post-image**
lines (the file as it exists at envelope `head_sha`), 1-indexed inclusive. On
GitHub that is the **RIGHT** side of the pull-request diff. **380.2** posts
`POST /repos/{repo}/pulls/{n}/reviews` after a successful summary comment, with
those coordinates and `commit_id = head_sha` — never the live PR head. **380.3**
locks the remaining failure modes: replay after a 5xx review, dual-surface
envelopes, review errors that must not `disable_link`, a vanished envelope at
send time, and the projector kind-split. The Cluster 379 summary comment path
is unchanged.

---

## What Maidan reads from the envelope

Maidan is a **tolerant reader**. The result is opaque JSON owned by the producer;
Maidan parses only the fields it routes on and ignores everything else. Adding a
field never breaks delivery.

| Field | Required | How Maidan uses it |
|---|---|---|
| `schema` | yes | Envelope discriminator. Must be `pi.waiter.result/1`. An unrecognized value means no delivery is attempted. |
| `result_kind` | yes | Which producer shape this is, e.g. `pi.review.result/1`. Recorded; the list facet shipped in Cluster 381 (see "Discoverability"). |
| `status` | yes | Delivery happens only on `reviewed`. Any other value delivers a short **Maidan-authored** failure notice instead — never silence, never a clean pass. |
| `deliver_to` | no | The routing list. Absent or empty is **valid and normal**: thread-only, delivered nowhere. |
| `rendered` | on `reviewed` | The delivery body. Producer-authored trusted markdown. |
| `summary` | on `reviewed` | One line. The Slack body and the notification title. |
| `view_in_pi` | no | A backlink appended to every delivery. |
| `pr` | no | A human back-reference echoed into the delivered body. |
| `head_sha` | for inline comments | GitHub `commit_id`. 40- or 64-char hex. Absent or unusable ⇒ no inline review (the 379 summary comment still posts). **Never resolved from the live PR head.** |
| `findings` | no | Two readers. Cluster 380 inline comments: each usable finding needs `file`, `body`, and `line_range`. Cluster 383 close-gate: any finding with `severity` exactly `critical` on a reviewed `pi.review.result/1` from a review-skilled producer writes Cluster-375 `request_changes` (and arms `k=1` if unset). A critical finding without file/body/`line_range` still arms the gate. Other finding fields stay in the stored JSON unread. |

`run_id` is **not** a delivery-routing field. Delivery parse still ignores it.
Cluster 387 homes the producer's string as `parent_run_id` on the thread — see
[Run lineage](#run-lineage-cluster-387). Everything else in the envelope —
`corroboration`, `per_seat`, `seats`, `cost_usd`, `duration_secs`, `sandbox`,
`finding_count`, `diff_available` — is carried through untouched. Maidan does
not interpret those for delivery.

**The canonical `Finding` wire shape is the producer's and does not change.**
Maidan stores the envelope byte-for-byte. Cluster 380 *projects* `file` /
`line_range` / `body` for the GitHub review POST. Cluster 383 reads
`severity` for the close-gate adapter only. Cluster 387 reads `run_id` for
lineage only.

---

## Run lineage (Cluster 387)

Wave 2 #28's lineage half. pi's waiter envelope already carries `run_id` (and
`view_in_pi`). Until this cluster that string had **no home in Maidan**. The
field **accepts the producer's value** — Maidan does not mint a parallel id.
The authoritative fixture value is `aa4dc966-0e09-44c3-b7a5-2d048b48b301` in
`crates/maidan-types/tests/fixtures/pi_waiter_result_v1.json`.

`run_id_from_payload` extracts a string (trim; empty / whitespace / longer
than 256 bytes → ignored). It does **not** require `schema = pi.waiter.result/1`.
`parse_waiter_result` still ignores `run_id`, so delivery routing is unchanged.

`set_thread_result` (REST `PUT /threads/:id/result` and the MCP tool) homes
the value as `parent_run_id` when present — best-effort, so a lineage hiccup
never undoes a stored result. A caller can also set it directly.

| Surface | Capability |
|---|---|
| `PUT` / `GET` / `DELETE /threads/:id/lineage` | write/delete = `thread:transition`; get = `workspace:read` |
| `GET /workspaces/:id/run-threads?parent_run_id=` | `workspace:read` (private-channel rows the caller cannot access are dropped) |
| `GET /workspaces/:id/run-occupancy?parent_run_id=` | `workspace:read` (empty / unknown run → zeros, not 404) |
| MCP `set_thread_lineage` / `get_thread_lineage` / `list_run_threads` / `get_run_occupancy` | the twins of those routes |

Nested occupancy is the two-clocks partition (`queued` / `claimed` / `working` /
`blocked`) of every **open** workspace thread that shares `parent_run_id`.
F7 mute (`maidan_thread_mutes`) is a different table and is **not** consulted —
a muted nested thread still counts.

**Still open on Wave 2 #28:** follow a member's occupancy; a manager digest.
Those are later slices, not this cluster.

---

## Inline findings (Cluster 380)

**Frame of reference (pinned 380.1).** `findings[].line_range` is a 1-indexed
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
successful GitHub summary comment (Cluster 380.2). A missing `head_sha`, no
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
on that commit); the 379 summary comment is the object that updates in place.

---

## The `deliver_to` grammar (pinned)

An array of target objects. Each names a `surface` plus per-surface detail:

```json
[
  { "surface": "github", "repo": "beatgig/bgv3", "pr": 3915 },
  { "surface": "slack",  "channel": "C0123ABCDEF" }
]
```

### Rules

1. **`surface` is a lowercase identifier.** Known values today: `github`, `slack`.
2. **An unknown `surface` is skipped with a recorded warning, never an error.**
   Partial delivery is the model: one target failing or being unroutable must not
   sink the others.
3. **`github` requires `repo` (`owner/name`) and `pr` (an integer issue or PR
   number).** Delivery is a PR/issue comment.
4. **`slack` requires `channel`, and it MUST be a channel ID** (`C…`/`G…`), not a
   `#name`. Names are mutable and ambiguous, and an allowlist keyed on a mutable
   name is not an allowlist.
5. **An empty or absent list delivers nowhere.** This is a valid, supported
   outcome — not a misconfiguration.
6. **New surfaces add new object shapes.** Rule 2 makes that backward-compatible:
   an older Maidan skips a surface it does not know.

### Where the producer gets it

The task author sets the routing intent in the seeding message's metadata under
`pi.deliver_to`; the producer carries it through opaquely and echoes it into the
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
  `github:beatgig/bgv3` or `slack:C0123ABCDEF` once, over
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

### What this means for the producer

**A perfectly correct `deliver_to` can still deliver nowhere.** That is not a
producer bug and must not be reported as a failure. Treat "delivered nowhere" as
a normal outcome; the per-target disposition is readable from the delivery-status
API below, and an operator — not the agent — fixes an unblessed target.

---

## Per-surface behaviour

### GitHub

- Body is `rendered` verbatim. GitHub renders GFM, so the producer's markdown
  arrives as written.
- A `view_in_pi` backlink is appended.
- Truncated with an explicit marker if it would exceed GitHub's comment ceiling
  (65536 characters). **Do not assume `rendered` arrives whole** — put the
  load-bearing content early and rely on the backlink for the rest.

### Slack

- **Slack does not render GFM.** Slack speaks *mrkdwn*: `*bold*` not `**bold**`,
  `<url|text>` not `[text](url)`, no headings, no tables. Posting `rendered`
  verbatim would ship visibly broken output.
- So Slack receives `summary`, a compact Maidan-projected digest, and the
  `view_in_pi` link — **not** `rendered` as-is. A producer that wants precise
  Slack formatting should make `summary` carry the weight.

### Both

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

`result_kind` is a **namespaced string**, not a closed enumeration. Pi publishes
`result_kind = "pi.review.result/1"` inside `schema = "pi.waiter.result/1"`; a
future producer ships a new string and the facet works without a server change.
The old guess of a closed enum (`decision|plan|merge_authorized`) is not the
wire vocabulary and is not a filter value.

List (not message-FTS) surfaces, `workspace:read`:

- REST: `GET /workspaces/{id}/results?result_kind=pi.review.result/1` (`limit`
  optional, default 50, clamp 1–500). Omit `result_kind` to list every
  non-tombstoned result the caller can access.
- MCP: `list_thread_results` with the same optional `result_kind` / `limit`.
  Workspace comes from the token; there is no `workspace_id` argument.

Match is exact on the indexed string. `pi.review.result` and
`pi.review.result/10` do not hit `pi.review.result/1`. Private-channel rows the
caller cannot access are omitted. This is not `GET /workspaces/{id}/search`
and not the ADR JSON convention `"kind": "decision"` in
[Integration.md](Integration.md#decision-records).

## Close-gate (Cluster 383)

A reviewed `pi.review.result/1` whose `findings` contain any
`severity == "critical"` is a Cluster-375 `request_changes` from a
review-skilled producer (`REVIEW_SKILL = "review"`). If the thread has no
requirement, Maidan arms `k = 1` so the existing close-gate refuses
`closed` until a human who is neither owner nor assignee approves.

This is an adapter, not a new gate:

- Warning-only / wrong `result_kind` / not `reviewed` / unskilled producer
  → no-op.
- An existing `k` is left alone.
- A clean re-review does **not** auto-approve.
- Empty `deliver_to` still arms — the room blocks the land even when
  nothing is posted externally.
- The GitHub review posted by Cluster 380 stays `event: COMMENT`. The
  room gate is the land decision; the PR is not REQUEST_CHANGES.

`PUT /threads/:id/result` and MCP `set_thread_result` arm on the write
path. The `ThreadResultSet` bus consumer arms again (every-replica /
replay). Both are idempotent.

## Delivery status

Per-thread delivery state (one row per target: disposition, external reference,
last error, attempt count) is readable over REST and MCP, with an operator replay
action. A producer can confirm where its result actually landed without scraping
the external surface.

---

## What Maidan will never deliver

- Raw agent output of any kind — seat stdout, un-rendered model text, tool
  transcripts. **Only `rendered`, `summary`, and structured `findings`.**
- Anything at all when `status` is not `reviewed` (beyond the failure notice).
- Anything to a target the workspace has not blessed.

Maidan does not run seats, does not render reviews, and does not become a
CI product. Cluster 383 reads `findings[].severity` for the close-gate
adapter only (`critical` → Cluster-375 `request_changes`); it does not
judge finding bodies or invent a land vocabulary. GitHub review `event`
stays `COMMENT` (Cluster 380). It delivers trusted bytes to blessed
surfaces, durably, once.

---

## Open requests to result producers

Two of three are now carried; one remains:

1. ~~**`head_sha`** — the commit the review was computed against.~~ **Carried and used (Cluster 380).** Present on
   `pi.waiter.result/1` (fixture lock). Maidan passes it as GitHub's `commit_id`
   rather than resolving the PR head at delivery time.
2. ~~**The frame of reference for `line_range`.**~~ **Pinned (380.1).** File-absolute
   **post-image** lines at `head_sha`, 1-indexed inclusive, GitHub **RIGHT**. See
   [Inline findings](#inline-findings-cluster-380).
3. **Call `report_usage`** with the run's `cost_usd` and `duration_secs`. Maidan
   ships a per-task token/USD/turn/wall budget envelope that stops a run when it
   is exceeded; a producer that reports its spend only inside an opaque result
   leaves that envelope blind. Self-reporting through the ledger API is the
   supported path — Maidan deliberately does not fold an agent-declared cost out
   of a result payload into its own billing basis.

## Versioning

The grammar above is **frozen at `pi.waiter.result/1`** (confirmed Cluster 379).
Additive fields are free. A change to the meaning of an existing field, or to the
`deliver_to` shape, requires a new `schema` value — Maidan will route on the
discriminator and an unrecognized one is inert rather than mis-delivered. The
Maidan-side lock is `crates/maidan-types/tests/fixtures/pi_waiter_result_v1.json`:
a producer-side grammar change breaks that test.
