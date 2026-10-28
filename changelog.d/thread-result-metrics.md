### Thread results by harness, kind, status and verdict

- **Added:** `maidan_thread_results_total{harness, kind, status, verdict}` counts every stored thread result, so a swarm's verdict mix (pass or fix, per Codex, Claude or Grok red-team) and outcomes can be charted without reading results. Every label is a closed set read from the result's `result_kind`, `status` and the report's `VERDICT` line; anything else reads as `other`, so no free text, id or model name becomes a series (ENG-6218).
