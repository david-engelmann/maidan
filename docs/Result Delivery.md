# Result delivery — the external last mile

**Audience:** authors of agents that write a structured result to a Maidan thread
and want it to reach an external surface (a GitHub PR comment, a Slack message).
The first such producer is **pi**, whose code-review waiter publishes a
`pi.waiter.result/1` envelope via `set_thread_result`.

**This page is the interface contract between a result producer and Maidan.**
Read it alongside [Integrating with Maidan](Integration.md).

---

## Status — read this first

**Not built yet.** This page pins the grammar so producers and Maidan agree
*before* the code lands; it is a specification, not a description of `main`.

| Piece | State on `main` today |
|---|---|
| `set_thread_result` / `get_thread_result` / `ThreadResultSet` event | **Shipped** (Clusters 234–236). A result is durable and observable. |
| Slack + GitHub connectors (post, link tables, link management) | **Shipped** (Clusters 307–312, 346, 349.4). |
| Durable retrying delivery queue + DLQ + replay | **Shipped for mail and webhooks** (Clusters 50, 304–306). Projector egress does **not** use it yet. |
| A `ThreadResultSet` handler that delivers to `deliver_to` | **Not built.** Planned as Clusters 377–381 — see [Open Work](Open%20Work.md). |

**Interim contract:** a producer may set `deliver_to` today. Maidan stores it as
part of the opaque result and **takes no action on it**. Nothing breaks; nothing
is delivered. External delivery stays the producer's own problem (for pi, the
soundcheck renderers) until the clusters below land. Write `deliver_to` now so the
data is already correct when delivery turns on.

---

## What Maidan reads from the envelope

Maidan is a **tolerant reader**. The result is opaque JSON owned by the producer;
Maidan parses only the fields it routes on and ignores everything else. Adding a
field never breaks delivery.

| Field | Required | How Maidan uses it |
|---|---|---|
| `schema` | yes | Envelope discriminator. Must be `pi.waiter.result/1`. An unrecognized value means no delivery is attempted. |
| `result_kind` | yes | Which producer shape this is, e.g. `pi.review.result/1`. Recorded; also the search facet (see "Discoverability"). |
| `status` | yes | Delivery happens only on `reviewed`. Any other value delivers a short **Maidan-authored** failure notice instead — never silence, never a clean pass. |
| `deliver_to` | no | The routing list. Absent or empty is **valid and normal**: thread-only, delivered nowhere. |
| `rendered` | on `reviewed` | The delivery body. Producer-authored trusted markdown. |
| `summary` | on `reviewed` | One line. The Slack body and the notification title. |
| `view_in_pi` | no | A backlink appended to every delivery. |
| `pr` | no | A human back-reference echoed into the delivered body. |
| `findings` | no | Stored and forwarded verbatim. Read only by the (parked) inline-comment path. |

Everything else in the envelope — `corroboration`, `per_seat`, `seats`, `run_id`,
`cost_usd`, `duration_secs`, `sandbox`, `finding_count`, `diff_available` — is
carried through untouched. Maidan does not interpret it.

**The canonical `Finding` wire shape is the producer's and does not change.**
Maidan stores and forwards it byte-for-byte.

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
  `github:beatgig/bgv3` or `slack:C0123ABCDEF` once. A target absent from the
  allowlist is **skipped with a recorded warning** — the same treatment as an
  unknown surface.
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
  and replay it. A misconfiguration class error (401/403/404) disables the target
  loudly rather than retrying forever.
- **A non-`reviewed` status is surfaced as a failure**, using `status` alone. It
  is never rendered as a passing review and never quietly skipped.
- Every delivery attempt is audited.

## Discoverability

`result_kind` is a **namespaced string**, not a closed enumeration, and is
indexed as a search facet — so `pi.review.result/1` results are findable without
Maidan needing to learn a new vocabulary word per producer.

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

Maidan does not run seats, does not render reviews, does not judge findings, and
does not become a CI product. It delivers trusted bytes to blessed surfaces,
durably, once.

---

## Open requests to result producers

Three things Maidan needs that the current envelope does not carry:

1. **`head_sha`** — the commit the review was computed against. GitHub anchors
   inline review comments to a commit; resolving the PR head at delivery time can
   anchor to a *newer* commit than the producer actually reviewed, misplacing
   every comment. **Inline per-finding PR comments are parked until the envelope
   carries this.** The single summary comment does not need it.
2. **The frame of reference for `line_range`** — file-absolute post-image lines,
   or diff-relative? Also needed for inline comments.
3. **Call `report_usage`** with the run's `cost_usd` and `duration_secs`. Maidan
   ships a per-task token/USD/turn/wall budget envelope that stops a run when it
   is exceeded; a producer that reports its spend only inside an opaque result
   leaves that envelope blind. Self-reporting through the ledger API is the
   supported path — Maidan deliberately does not fold an agent-declared cost out
   of a result payload into its own billing basis.

## Versioning

The grammar above is **frozen at `pi.waiter.result/1`**. Additive fields are free.
A change to the meaning of an existing field, or to the `deliver_to` shape,
requires a new `schema` value — Maidan will route on the discriminator and an
unrecognized one is inert rather than mis-delivered.
