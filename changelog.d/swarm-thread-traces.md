### Swarm observability: thread traces and claim latency

- **Added:** Each thread's stages (claim, acknowledge, result, review verdict, delivery, landed) record a `maidan.thread` span on one trace per thread; the trace id is the thread's id, so a seat's harness can join with `traceparent: 00-<thread id hex>-<span>-01`. Span attributes come from a fixed allowlist of names and closed values; event text never reaches a span.
- **Added:** `maidan_thread_claim_duration_seconds{harness,kind}`, the time from a claim's acknowledgement to its stored result.
- **Added:** `maidan_result_deliveries_total` carries a closed `surface` label (github, github_branch, slack, linear, other) and now counts delivered, failed and skipped outcomes per surface.
