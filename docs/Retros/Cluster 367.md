# Cluster 367 retro — Wave 2 #15: the human work console

Wave 2 flips Handoff rule 8 for the Work tab: a human now *inhabits* the workplace
loop through the vanilla `/ui` (still no SPA). #15 is "three bullets" — B2 a **Work
tab**, B11 a **Prefs console**, and B3 a **looking glass**. All the machinery
already shipped in Wave 1; this cluster is pure surfacing.

## What shipped

- **367.1 (#722) — the Work tab.** Pick a channel → its queue depth (Cluster 224) +
  occupancy (Cluster 351); browse threads; Inspect a thread for its result
  (Cluster 234) + DAG dependencies (Cluster 217). Lists the workspace's task
  schedules (Cluster 226). Five session-proxied `/ui/api` reads + a Playwright spec.
- **367.2 (#723) — the Prefs console.** Delivery mode (256), delivery email (250),
  muted notification kinds (242), followed channels + threads (245) — self-service,
  self-only. Twelve `/ui/api` routes (5 reads + 7 writes).
- **367.3 (#724) — the looking glass.** A read-only explorer: events by kind,
  thread by id, artifact by sha (404 → not-found), federation peers. One new
  `/ui/api` read (artifact meta) + a Playwright spec.

## Decisions

- **Sequential sub-PRs, not independent.** Unlike Wave 1 #14 (four disjoint files →
  four parallel PRs), all three tabs edit the single `static/index.html`. Stacking
  them would mean same-file rebase conflicts on every merge, so each merged before
  the next branched off `main` — serial, but clean.
- **Read/observe first; agent actions stay agent-side.** The Work tab surfaces the
  task loop for a *human* to watch and manage; claim-next needs `thread:transition`
  (an agent capability a session usually lacks), so it's left observe-only. The room
  is designed around intent→decompose→park→land, not a topology of controls.
- **Off the OpenAPI/capability-map contract.** Every `/ui/api` route reuses a tested
  handler under the session-or-bearer middleware and stays out of the OpenAPI doc +
  capability-map (the Cluster-251 precedent), so the bijection + matrix contracts
  needed no entries — the wiring is guarded by `ui_js_contract` + the underlying
  handlers' own e2es.
- **Guard density by testability.** The Work + looking-glass tabs drive from the
  workspace input (a bearer token works), so they got Playwright specs. The Prefs
  tab is self-only via `sessionMemberId` — the bearer-token Playwright fixture can't
  sign in — so it leans on the static `ui_js_contract` guard + the four backend REST
  e2es (`notification_prefs` / `delivery_mode` / `email` / `follows`).

## Surprises

- **Nothing broke the existing Playwright specs.** Adding three tab buttons was a
  worry (the a11y roving-tabindex, the smoke spec) — but no spec asserts a tab
  count, and `initTablist` wires the new buttons' ARIA + keyboard nav automatically.
- **`escapeHtml` was already there.** Interpolated result JSON and event kinds go
  through it; message bodies and ids use `textContent`, so no new XSS surface.

## Test evidence

- `ui_js_contract`: `ui_js_wires_work_tab` / `_prefs_tab` / `_looking_glass_tab`
  (the required-job static guards — loaders defined + invoked, tab-switch wired,
  panels present).
- Playwright: `work.spec.ts` (channels load, queue depth + threads render, Inspect
  shows detail), `glass.spec.ts` (unknown sha → not-found).
- The surfaced endpoints are all backend-tested from Wave 1.

## Forward look

**Wave 2 #15 is complete.** Deferred: human claim-next (needs a session
`thread:transition` grant); a live-refreshing Work console (the WS live-refresh from
Cluster 153 could bump queue depth); richer DAG visualization; skills management in
the Work tab. **Next: Wave 2 #16** — a waiting-on-you inbox (G15 + G9: assigned-to-me
+ open gates + required-human mentions, each with an SLA).

## Acknowledgements

Three sequential PRs (#722 → #723 → #724) plus this retro, on the Cluster-251
`/ui/api`-proxy + `ui_js_contract` + Playwright-harness patterns.
