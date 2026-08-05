# Cluster 152.0 retro — lean HTTP context pack + snippet-only search

> Tag **`v152.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Token-efficiency part 2 (arc item B1) — REST parity with Cluster 151.

## What shipped

- **HTTP context pack edits are lean by default.** `ThreadContext.message_edits`
  is now `Vec<MessageEditView>` — `{id, message_id, editor_id, edited_at}` with
  **optional** `body_before`/`body_after`. Bodies are omitted unless the new
  **`include_edits=true`** query param is passed on `GET /threads/:id/context`
  or `GET /workspaces/:wid/context`. Matches the MCP default from 151.
- **`snippet_only=true` on `GET /workspaces/:wid/search`.** Drops the full
  message `body` from each hit. Lexical hits keep their FTS `snippet`; semantic
  hits (empty snippet) get a UTF-8-safe truncated `body` prefix
  (`SearchHit::into_snippet_only`, `SNIPPET_FALLBACK_BYTES = 240`) so they still
  carry locatable content. Default response unchanged.
- OpenAPI registers `MessageEditView`; the two context query DTOs gain
  `include_edits`, `SearchQuery` gains `snippet_only`.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| n/a | Default-lean search | Kept opt-in — dropping bodies by default would surprise existing search consumers; the context-pack change is where lean-by-default is worth the break. |
| Future | Unify MCP vs. server context builders | Still two implementations; a real refactor, out of scope for a token-efficiency pass. |

## Surprises

- **The semantic snippet asymmetry.** `body`/`snippet` looked redundant until the
  backends showed semantic search sets `snippet = ''` (postgres `'' AS snippet`,
  sqlite `String::new()`) and leans entirely on `body`. A blind "drop body"
  would have returned empty semantic hits. The fix — synthesize a truncated
  snippet from the body when the snippet is empty — keeps `snippet_only`
  meaningful across all three modes (lexical / semantic / hybrid).

## Decisions

- **`MessageEditView` with `Option` bodies** over the alternatives (clearing
  `MessageEdit`'s String fields to `""`, or switching to untyped
  `Vec<Value>`). It keeps a clean, honest OpenAPI schema: the bodies are
  *optional*, present exactly when `include_edits=true`.
- **Context edits lean-by-default, search snippet opt-in.** The pack is packed
  into prompts (token cost is the whole point), so lean is the right default;
  raw search results are consumed more variably, so the safe default there is
  the status quo.

## Capability table extension

| Capability | Where |
|------------|-------|
| Lean HTTP context edits + `include_edits` opt-in | `crates/maidan-server/src/thread_context.rs` |
| `snippet_only` search | `crates/maidan-server/src/routes/search.rs`, `crates/maidan-search/src/hit.rs` |

## Risks identified + still open

- **Low.** Context-pack default drops edit bodies (opt-in restores; flagged
  **Changed**). `MessageEditView` is a strict superset of the lean shape.
  Search is fully backward compatible. No consumer read `.message_edits` typed
  outside the builder.

## Forward look

Token-efficiency (arc item B1) is now complete across MCP (151) and REST (152):
both context-pack surfaces + search have opt-in / default token-lean modes. Per
the user's instruction to run all three next-arc lanes in order, next is the
**live-updating `/ui` thread view** (route the WS message/reaction/pin frames —
today dumped as raw log lines — into `loadMessages`), then the **`request_client`
GET-stream fix + a real caller**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
