# Cluster 127.0 retro — Backlog reconciliation

> Tag **`v127.0.0`**. Phase XXIV (post-gate hardening). Docs-only. No new gate tag.
> First cluster of the "plan the remaining work" sweep (127 = reconcile, then
> 128–130 hardening, 131–132 optional).

## What shipped

- **`Remaining Work.md` + `Open Work.md` reconciled against code at v126.**
  ~11 entries the docs listed as open were verified already-shipped and struck
  with the shipping cluster + evidence: group DMs (97), presence/typing (103),
  per-model embedding tables (86), `sqlite-vec` (85), schema-parity tests,
  cosign signing, bootstrap compile-time strip (91), SQLite delivery cursor,
  Helm prod profiles, context thread cursor, Web UI tabs.
- **The stale `Open Work` tail fixed** — it still claimed "latest tag v76.0.0 /
  active cluster 78" and listed phantom deferrals (OpenAPI map, OTLP dashboards,
  sqlite-vec). Now reflects v126 + the genuinely-open set.
- **§4 Slack parity classified** — product/UI (complete backends) vs out-of-scope
  vs backend-tractable, so future planning doesn't mistake UI work for gaps.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| 128–130 | The genuinely-open hardening items | This cluster only corrects the record; fixes are sequenced separately. |
| 131–132 (optional) | Unify delivery tables; global admin audit API | Real but lower-value; confirm before doing. |

## Surprises

- **The backlog was ~85% phantom.** Of the items the docs presented as open,
  the large majority had already shipped — a direct consequence of the docs
  being updated at retro time but the "open" lists not being re-verified against
  code. The verification pass (parallel code reads, not doc reads) was the whole
  value.
- **Two docs disagreed with each other.** `Remaining Work` §1 said per-model
  embedding tables + sqlite-vec shipped, while §3 listed them as deferred; §7
  claimed the SQLite delivery cursor was a no-op while it's fully implemented.
  Cross-doc contradictions are the tell-tale of drift.

## Decisions

- **Verify against code, never against the other doc.** Every correction cites a
  shipping cluster + file. The reconciliation note in each doc says it was done
  at v126 so the next reader knows the as-of point.
- **Don't delete history — strike with evidence.** Struck entries keep a
  one-line "Closed (N): …" so the record shows what shipped, not just that it's
  gone.

## Capability table extension

| Capability | Where |
|------------|-------|
| Backlog reconciled against code (v126) — trustworthy open-work list | `docs/Remaining Work.md`, `docs/Open Work.md` |

## Risks identified + still open

- **Drift will recur** unless the "open" lists are re-verified at each retro, not
  just appended to. The reconciliation note + the per-entry shipping-cluster
  citations make the next audit cheaper.

## Forward look

With an accurate backlog, the hardening sweep is well-scoped: A2A delivery
robustness (**128**), error-visibility + bounded buffers (**129**), test-coverage
uplift (**130**); optional unify-delivery / admin-audit (**131–132**).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
