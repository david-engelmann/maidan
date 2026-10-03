# R1: Maidan's context surfaces and cacheability

> Research input to [Context Economics](../../Context%20Economics.md), dated 2026-10-01. Kept as evidence: every claim carries its source and access date. The fetched pages and clones it cites were working copies and are not committed; the URLs and `repo@commit` citations make each claim re-checkable.


From the codebase audit at bd360306, 2026-10-01. Evidence is file:line on main.

**Rechecked on `main` at `5ec089b2` (2026-10-03).** Every finding below still holds. The MCP catalog has since grown from 200 to 235 tools (#1226, #1227), and the generated reference from 118,145 to 133,606 bytes, so `tools/list` is now about 105 KB compact (estimated with the same ratio), roughly 26 to 30 thousand tokens. No surface has gained `ttlMs`, `cacheScope` or `server/discover`; the pack's ties and REST/MCP split are unchanged; nothing outside tests reads the usage ledger.

## Serialization
- serde_json `preserve_order` is off (Cargo.toml:36, which has `float_roundtrip`; there is no indexmap in Cargo.lock). Every `json!`/`Value` object therefore has alphabetically sorted keys (a BTreeMap). Structs serialize in field order.
- REST packs serialize structs and come out in field order. MCP packs are built with `json!` (`maidan-mcp/src/context.rs:274-287`) and come out alphabetical. **REST and MCP give different bytes for the same state**, so their snapshot shas differ.
- MCP tool output is compact `to_string`, a single text part (`tools/mod.rs:1071-1079`), with no `structuredContent`.
- Precedents to reuse: `canonical_json` (`signed_export.rs:90-130`), `content_hash_of` (`event_chain.rs:271`), the Agent Card's ETag (`card.rs:172-215`). No context endpoint has an ETag.
- No response body carries a request id or a random value. Request ids and the room LSN are headers only.

## Thread context pack (REST `GET /threads/{id}/context`; MCP `get_thread_context`)
- **Ordering is deterministic except:**
  - artifacts: a `HashSet`, then a stable sort by `created_at` only, so ties come out in random order (`thread_context.rs:218-233`, `context.rs:78-99`);
  - references: `ORDER BY created_at` with no tiebreaker (`refs.rs:78,98,119`);
  - accepted decisions: `produced_at DESC` with no tiebreaker (`thread_results.rs:80,111`).
- **Key order, REST:** `workspace_id, channel_id, thread, messages, message_edits, references, artifacts, fsm, glossary, elision, parent_grounding, accepted_decisions, change_requests, next_message_cursor`.
- **Key order, MCP (alphabetical):** `accepted_decisions` comes first, and it prepends whenever a sibling thread closes (`LIMIT 10`, newest first, `pack.rs:127`).
- **`thread` is volatile:** `updated_at` is bumped on every post (`messages.rs:131,303`) and on every claim, renew or release. Its lease fields change with each claim.
- **The stable grounding comes after the messages:** glossary, parent ask and decisions. Neither layout is stable-first.
- **Size:** 19,802 B (about 4,951 tokens) for a 40-message thread (`Benchmark.md:61-80`). Each message carries 237 B of fixed JSON, with `thread_id` repeated on every message.
- **Default page:** the oldest 100 messages (cap 500).
- **`token_budget`:** a chars/4 estimate (`pack.rs:29,37`). Its fold keeps the opener plus the recent tail (`pack.rs:259-330`), so the kept set and the elision text change with every new message. That is hostile to prefix caching.

## Other surfaces
- **as_of replay** (Cluster 326): `thread` is the live row. REST overwrites only `state` (`thread_context.rs:476`) and MCP not even that (`context.rs:342,387`), so a later post changes the bytes of an as-of pack. `Integration.md:816-817` promises "exactly as it stood".
- **Snapshot** (329-330): the sha256 of the serialized pack bytes (`routes/thread.rs:185-189`, `tools/snapshot.rs:23-31`). A REST and an MCP snapshot of the same state get different shas.
- **Workspace pack:** REST order is `workspace, channels, threads, glossary`; MCP is alphabetical. Up to 50 threads × 500 messages.
- **`get_waiting_inbox`:** ages every item against `Utc::now()` (`member.rs:309`), so it is nondeterministic. **`get_manager_digest`:** echoes `since = now−7d`. **`catch_up`:** carries `head_lsn` and `room_lsn`, which other tenants move.
- **`claim_next` and `wait_for_*`:** volatile by design.
- **Stable per release:** `llms.txt` (4,571 B), MCP `initialize` instructions, the Agent Card (ETag, `max-age=300`, embeds the version).

## MCP `tools/list`
- **200 tools**, filtered by the token's capabilities (`tools/mod.rs:59-74`), so agents with different capabilities get different tool prefixes.
- Order is source order. Keys within each tool are sorted.
- **Size:** 92,688 B compact, about 23k tokens at chars/4 (likely 26-31k), of which 37.5 KB is descriptions and 43.0 KB schemas.
  - worker capability set: 179 tools, 83,576 B
  - read-only: 110 tools, 45,728 B
  - the 7-tool hero loop: 6,829 B
  - the `llms.txt` loop: 14 tools, 12,629 B
  - largest single tool: `get_thread_context`, 2,377 B
- **Churn:** 76 commits touched `catalog.rs` in 30 days (11 in the last 7). Each changes the tools prefix.
- MCP `ttlMs` and `cacheScope` were declined (Open Work).

## Determinism tests
- Two-call byte identity is checked only by snapshot dedup on a one-message fixture: `context_snapshot_e2e.rs:156-170` and the MCP `server.rs:8332-8343`.
- The goldens normalize timestamps and ids. The catalog contract checks tool names only.
- There is no golden for pack bytes, `tools/list` bytes, REST/MCP parity or tie order.
- `docs.yml` regenerates `mcp-reference.md` with no drift check.

## Usage ledger (PayerStamp, migration 0102)
- **Stores** `TokenUsage {input, output, cache_read, cache_write}`, four reporter-supplied micro-USD-per-million prices, `usd_micros` (validated against them), a free-form `model`, `turns`, the claim lease, the thread and the workspace.
- **Written through** REST `POST /threads/{id}/usage` and MCP `report_usage`, via `report_accounted_usage` (`postgres/budget.rs:184-300`). Writes are fenced by the claim lease and idempotent.
- **Budgets:** `used_tokens` adds `tokens.total()`, the sum of all four tiers (`budget.rs:224`), so `max_tokens` counts cache reads one-for-one.
- **Nothing reads the rows.** `list_thread_usage_ledger` is used only in tests. The ledger is not in metrics, the UI, the manager digest, `/operator` or the SDKs. `UsageReported` events carry the full stamp.
- **Hit rate and dollars saved can be computed per row, with caveats:**
  - `input` semantics are undefined (OpenAI's prompt tokens include cached tokens, which would be double-counted);
  - one `cache_write` rate covers both 5-minute and 1-hour writes;
  - there is no provider field;
  - a report is a heartbeat, not a request;
  - the examples report `cache_read: 0`.

## Coordination hooks
- **`claim_next`:** one SQL for both scopes. It orders by `(priority + hours_waiting) DESC, created_at, id` with `SKIP LOCKED` (`claim_next.rs:104`), and has no affinity term and no budget check.
- **Leases:** the default of 600 s is longer than the 5-minute cache TTL.
- **`wait_for_ready`:** already workspace-wide when `channel_id` is omitted (`catalog.rs:1487-1497`). Queue depth and occupancy are per channel only.
- **Recipes fan out** children that share one `parent_grounding`, a natural shared prefix.
- **No model or provider field** exists on members, apps, installations or tokens. Workable inputs: free-form skills, and `maidan_thread_workers.last_held_seq` (0124).
- **A per-workspace policy table** would follow the existing pattern (`0134_retention_policies.sql`).

## Docs
- Docs that promise fewer tokens: `Glossary.md:119-126`, `Claims.md:30`, `Benchmark.md` (6.8×), `Integration.md:812`, `Capabilities.md:795,1377`, `Framework Integrations.md:10` (the 7-tool loop).
- **No doc mentions provider prompt caching.**
- Seed `pack`/`prefix` inclusion was declined (`Decisions.md:1749-1786`).

## Other
- Maidan makes no LLM calls, only embeddings (batches of 32, no query cache).
- It stores no summaries. The closest things are thread results, 240-B decision excerpts, memory blocks (not in the pack), thread steer and seed-from-message.
