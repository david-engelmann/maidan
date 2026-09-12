# Cluster 378 retro — the trust boundary + the sender upgrade

Cluster 377 made projector egress *durable*. This cluster makes it **safe to aim**
and **safe to repeat**, which are the two things standing between the queue and a
result actually being delivered.

Three of the five gaps the result-delivery investigation found are closed here:

- **The confused deputy (gap 4).** A result's `deliver_to` is written by an
  *agent* — anyone holding `thread:transition` on the thread — while Maidan's
  connector credentials are operator-held and reach many repositories and
  channels. Routing straight off that list would let a compromised or merely
  buggy agent post under Maidan's identity anywhere the token reaches.
- **No update-in-place (gap 3).** `post_message` / `post_comment` returned `()`,
  so there was no `ts` and no `comment_id`, so a re-review could only ever stack
  a second comment on the PR.
- **Slack renders mrkdwn, not GFM (gap 5).** Posting `rendered` verbatim ships
  visibly broken output; and `rendered` quotes attacker-influenced code, so it
  can ping real humans from bytes an attacker chose.

**Nothing delivers a result yet.** This is the last cluster of foundation before
Cluster 379 wires the primitive: an allowlist nothing consults, senders nothing
updates with, and body rules nothing calls. That is deliberate — each piece is
proven on its own terms, so 379 composes rather than decides.

## What shipped

- **378.1 (#782) — the egress allowlist.** `maidan_egress_targets` (pg 0084 /
  sqlite 0083, `UNIQUE (workspace_id, surface, selector)`) + `AllowedEgressTarget`
  / `NewEgressTarget` + `allow_egress_target` (idempotent) /
  `list_egress_targets` / `revoke_egress_target` (workspace-scoped) /
  `is_egress_target_allowed`, both backends. Over `POST`/`GET
  /workspaces/:wid/egress-targets` + `DELETE …/:tid`, all **`token:admin`**, both
  mutations audited. **`deliver_to` selects; the allowlist authorizes** — and it
  defaults empty, so an unconfigured workspace delivers nowhere (the Cluster-371
  secret-broker fail-safe).
- **378.2 (#783) — the sender upgrade.** `ExternalRef` (`Slack { channel_id, ts }`
  / `Github { repo, comment_id }`) returned from both `post_*`, plus
  `SlackSender::update_message` (`chat.update`) and
  `GithubSender::update_comment` (`PATCH /repos/{repo}/issues/comments/{id}`).
  `post_message` gained `thread_ts`, so a re-delivery can reply *inside* the
  Slack thread it first posted in. A shared `call` helper now decodes Slack's
  `{ok, error}` envelope once instead of per method.
- **378.3 (#784) — `egress_body`.** Pure and unit-tested:
  `neutralize_github_mentions` (a prose mention wrapped in a code span),
  `neutralize_slack_mentions` (`<` of `<!…>`/`<@…>` escaped to `&lt;`),
  `truncate_with_tail` (GitHub's 65536-character ceiling, keeping the backlink),
  and a deliberately narrow `gfm_to_mrkdwn`. Composed by `github_comment_body`
  and `slack_message_body`, both taking plain strings so the module has no
  dependency on the 379.2 envelope parser.
- **378.4 — this retro + the doc-close.**

## Decisions

- **The authorization grain is coarser than the delivery grain, on GitHub only.**
  A delivery selector is `owner/name#123`; an operator blesses the
  **repository**. Per-issue blessing would technically work and be useless in
  practice — an operator ticket per PR. `EgressTarget::allowlist_selector()` is
  that projection, and the store test asserts the issue-qualified string is
  deliberately *not* an allowlist key, so nobody "fixes" the check into looking
  it up later. Slack has no such split: a channel id is already the unit an
  operator thinks in.
- **A selector must be an id, never a name.** A Slack `#channel-name` and a
  GitHub `owner/name#123` are both refused. The channel a *name* points at can
  change under the blessing, so an allowlist keyed on a mutable name is not an
  allowlist. The predicate is pure, unit-tested, and enforced in the **store**,
  so every write path inherits it and the route gets its `400` for free through
  `StoreError::InvalidInput`.
- **Write typed, read as stored.** `NewEgressTarget.surface` is an
  `EgressSurface`, so a surface this build cannot deliver to can never be
  blessed; the stored row's `surface` is text, so a row written by a *newer*
  build stays **listable** after a downgrade — otherwise an operator could not
  see the entry they need to revoke. That is Cluster 377.1's lesson (a row that
  cannot decode must still reach the human who has to act on it) applied to the
  read side. The two directions want different strictness, and saying so in the
  type is cheaper than a comment asking people to remember.
- **The whole allowlist surface is `token:admin`, reads included.** The allowlist
  is *policy*, not status. Letting a workspace-scoped token enumerate it would
  hand an agent the list of destinations worth aiming at. A producer that wants
  to know where its result actually landed reads the per-thread delivery status
  (Cluster 379.5) — the disposition, not the policy.
- **The allowlist id is a surrogate uuid.** A GitHub selector contains a `/`, so
  `DELETE …/{surface}/{selector}` is not routable. The operator's loop is list →
  bless → revoke-by-id, and `{tid}` matches the `{id}` convention the capability
  map already uses.
- **Allowlist reads stay on the primary.** The Cluster-265 control-plane
  carve-out: `is_egress_target_allowed` is an authorization check, and a lagging
  replica must not deny a blessing an operator just granted.
- **A post that succeeded is never reported as a failure.** Both `post_*` return
  `Result<Option<ExternalRef>, _>`. When a surface accepts the message but hands
  back no usable handle, the object *exists* — an `Err` would make the
  at-least-once worker retry and leave two comments on the PR, reintroducing at
  the decoding layer the exact duplicate-delivery harm the queue's dedup index
  exists to prevent. `Ok(None)` costs only the ability to edit it later, and the
  recovery path for a lost ref is the hidden body marker (379.4), never a
  re-post.
- **`ExternalRef` stores only its handle.** A delivery row already carries its
  `EgressTarget`, so `for_target(&target, handle)` rebuilds the rest. One text
  column instead of duplicating the repo and channel — the same "store the narrow
  thing, decode it back" move as `(surface, selector)`.
- **The GitHub ref carries no issue number.** `PATCH
  /repos/{repo}/issues/comments/{id}` does not take one, and carrying it would
  invite keying an update on the wrong thing. The wire test asserts the path has
  no issue number in it.
- **Mentions are defused with a code span, not an invisible character.** GitHub
  documents that a mention inside code does not notify, so the guarantee rests on
  a *rendering rule* rather than on a zero-width space or an empty HTML comment
  surviving whatever the sanitizer does this year. It is also visible: a reader
  sees the mention was defused instead of wondering why `@someone` never
  answered. Slack's `&lt;` escape is the same kind of choice — a documented
  unescape-on-render, so the reader still sees `<!channel>`.
- **`gfm_to_mrkdwn` is narrow on purpose.** It converts links, ATX headings and
  doubled emphasis — what Slack renders *wrongly* — and leaves single `*`/`_`,
  lists, quotes, code and tables alone. The rule that Slack receives a short
  producer-written `summary` (never `rendered`) is what keeps the job one line
  wide. A half-correct full converter applied to 4 KB of GFM would be worse than
  an honest narrow one applied to one line, and rewriting `*` would mangle
  `2 * 3`.
- **Truncation says so and keeps the link.** A body that silently lost its tail
  is worse than one that admits it. A budget too small for any content returns
  the notice and the backlink rather than a fragment that claims to be a review.
- **The projector egress kept its behaviour exactly.** The worker passes
  `thread_ts: None` and calls no `update_*`. Threading a linked thread's relayed
  messages under a parent would change Cluster 309's behaviour, and a result
  delivery replying in-thread is 379.4's call to make with a ref to reply under.
  All three mock senders assert `thread_ts == None`, so that stays true by
  construction rather than by intention.

## Surprises

- **The allowlist key is not the outbox selector, and 377.1 says it is.** That
  module's doc comment promised the `(surface, selector)` pair was "the pair the
  Cluster-378 allowlist will also key on" — true for Slack, wrong for GitHub. It
  only became visible while writing the store test. A cross-cluster promise
  written one cluster early is a guess, and this one was half right.
- **The honest sender return type has two layers, and the first cut had one.**
  `Result<ExternalRef, _>` looks right until a 201 arrives with a body that will
  not parse. *Did it land?* and *can we address it?* are separate questions, and
  collapsing them would have double-posted on every such response.
- **A code segmenter turned out to be the real primitive in 378.3, and it was not
  in the plan.** Each of the three body rules is wrong if applied inside a fenced
  diff, and all three needed the same answer, so "where does code start and stop"
  is what actually had to be built. Treating an *unclosed* fence as code is the
  small decision that keeps it conservative: the failure mode becomes "we left a
  mention alone that the renderer also leaves alone" rather than "we rewrote
  bytes a reader will see raw".
- **A weak test assertion failed for the wrong reason.** `!out.contains('b')`
  failed on a *correct* truncation, because the notice says "truncated **b**y
  Maidan". An assertion that happens to overlap the thing under test is worse
  than no assertion — it invites "fixing" working code. Replaced with an exact
  whole-output comparison.
- **Slack and GitHub disagree about what identifies a message, in a way that
  shaped the type.** Slack's `chat.update` needs channel **and** `ts` (a `ts`
  alone does not identify a message); GitHub needs repo and comment id and has no
  slot for the issue number. A flat `{id: String}` would have been smaller and
  wrong on both surfaces.

## Test evidence

- Store, both backends: `egress_targets` — deny-by-default on an empty
  allowlist; a near-miss matrix (wrong selector, wrong surface, and the same
  selector text under the *other* surface) proving the check is keyed on all
  three columns; cross-workspace isolation for the identical destination; the
  repo-covers-the-issue grain in both directions; an idempotent re-bless (same
  id, original `created_at`, still one row); the selector refusals as
  `InvalidInput`; and a workspace-scoped revoke (another tenant's id revokes
  nothing, a second revoke and an unknown id both report nothing removed).
  `dialect_parity` + `backend_parity` + `concurrent_migrations` green.
- Server: `egress_targets_e2e` **auth-enabled** (the surface is entirely
  `token:admin` and the point of an allowlist is who may change it, so a bypass
  run would prove nothing) — the operator's loop end to end, `400` on a
  name-not-an-id, a client error on an unknown `surface` at the extractor so the
  vocabulary cannot drift, `403` on all three routes for a
  `workspace:read`+`write` token, and a second test that a `token:admin` token
  cannot bless *another* workspace's target.
- Wire: `egress_wire_e2e` grew from 5 to 13 tests, driving the **real**
  `SlackWebClient` / `GithubApiClient` against a loopback server over all four
  calls — `POST /api/chat.update`; `PATCH
  /repos/acme/widgets/issues/comments/998877` with the `User-Agent` GitHub
  requires and no issue number in the path; `thread_ts` present when threading
  and *absent* (not null) when not, because Slack treats an explicit null as an
  error; the ref decoded from each response; both `Ok(None)` cases (a `ts`-less
  Slack success, and GitHub answering `{}` / `{"id": 0}` / `{"id": "998877"}`);
  and the update-side classification split — a 404 disables the link, a 403 with
  `retry-after` does not.
- Pure: `ExternalRef` round-trip through target + handle, and a malformed handle
  (`""`, `"nan"`, `"0"`, `"-5"`, `"12.5"`) rebuilding nothing. `egress_body` — 16
  units covering a team ping defused whole, mentions in spans and backtick and
  tilde fences untouched, six non-mention `@` shapes left alone (`a@b.com`,
  `5 @ each`, …), Slack broadcasts and group pings escaped while
  `<https://…>` autolinks are not, truncation on a multi-byte body (a
  byte-indexed cut would panic), the paragraph-break preference asserted exactly,
  GitHub's ceiling honoured for a 128 KB `rendered` with the link surviving, the
  narrow-by-design mrkdwn cases including what it refuses to touch, and a
  `<!channel>` **smuggled through `summary`** still defused — the producer is not
  the trust boundary.
- Regression: `egress_worker_e2e`, `egress_dlq_e2e`, `slack_egress_e2e`,
  `github_egress_e2e` all green with the three mock senders updated; the egress
  arc still delivers once, reschedules rather than drops, dead-letters an
  undecodable destination without a post, disables a misconfigured link, and
  leaves an unconfigured deployment alone.
- Contracts: `openapi_e2e` bijection, `http_capability_map_contract`,
  `http_openapi_capability_map_contract`, `http_capability_matrix_e2e`,
  `scripts/check-agent-contract.sh`. `mdbook build` with the linkcheck renderer,
  since `docs/Result Delivery.md` is published.

## Forward look

**Cluster 378 is complete.** A target must be blessed by an operator before
Maidan will post to it; a sender can say what it created and edit it later; and a
body arriving on either surface is mention-free, within the ceiling, and in that
surface's own markup.

Deferred (follow-ups): MCP twins of the allowlist routes — deliberately not
shipped, since an agent has no business editing the boundary that constrains it;
a `/ui` allowlist panel; wildcard or org-level selectors (`acme/*`), which are a
much weaker boundary that nobody has asked for; Slack Block Kit; and a wider
`gfm_to_mrkdwn`, which the "Slack gets `summary`" rule is meant to make
unnecessary.

**Next: Cluster 379 — the result-delivery primitive**, the producer's actual ask.
`maidan_result_deliveries` keyed `(thread_id, target_fingerprint)` (379.1); the
contract lock `parse_waiter_result` against the real fixture **committed** to the
tree, so a producer-side grammar change breaks a test (379.2); the
`ThreadResultSet` arm in `notification_router::route_event` that fetches, parses,
allowlist-checks and enqueues per target (379.3); idempotent update-in-place via
the stored `external_ref` with the hidden `<!-- maidan:result:<thread_id> -->`
marker as the recovery path (379.4); and `GET /threads/:id/deliveries` + replay +
MCP twins + an `audit::record` per attempt (379.5). See the "Result delivery —
the external last mile" section of [[Open Work]] and the pinned contract in
[Result Delivery](../Result%20Delivery.md).

## Acknowledgements

Three impl PRs (#782 the allowlist → #783 the sender upgrade → #784 the body
projection) + this retro, on the foundation-then-wire + new-route-preflight
patterns. All three rebased onto `main` twice as Cluster 377's PRs squash-merged
underneath them; each rebase was verified content-identical by diffing the old
and new deltas before pushing.
