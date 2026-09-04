# Cluster 352 retro — the HITL list (A2A `tasks/list` conformance)

Wave 1 #3. A human-in-the-loop **held gate** (Cluster 350) is durable and
queryable over Maidan's own surface, but an *external* A2A agent — one that
speaks only the A2A protocol — had no way to **discover** that a gate was
waiting for it. A2A's discovery primitive is `tasks/list`, and Maidan's
`282–289` A2A arc shipped it as a thin pass-through: no status filter, no
`input-required` visibility, no timestamp filter, a 200-item clamp, and
`application/json` instead of the spec's `application/a2a+json`. This cluster
brings `tasks/list` up to the live A2A 1.0.0 conformance bar **and** makes a
pending held gate show up in it — so an external agent can poll
`tasks/list?status=input-required` and find the gate it must answer.

## What shipped

- **352.1 (#628) — pending gates surface as `input-required` tasks.** A held
  gate has no row in `maidan_a2a_tasks` (it is a Maidan approval-gate, not an
  A2A task), so `tasks/list` never saw it. Rather than materialize a real task
  (which would violate A2A task-state monotonicity — tasks complete
  synchronously on send), `gate_as_task(&ApprovalGate) -> Task` **synthesizes**
  an `input-required` task view from the gate at read time. `dispatch_get_task`
  falls back to a gate lookup when no task id matches; `dispatch_list_tasks`
  leads the page with the workspace's pending gate-tasks. RBAC-checked
  (`ensure_workspace` + `ensure_thread_access`) so a caller only sees gates on
  threads it can reach.
- **352.2 (#629) — `status` filter + `pageSize` max 100.** `normalize_task_state`
  accepts kebab (`input-required`), enum (`TASK_STATE_INPUT_REQUIRED`), and
  bare (`INPUT_REQUIRED`) spellings → one canonical form, so `?status=` matches
  regardless of dialect. `pageSize` clamps to `1..=100` (the tree clamped 200;
  the live spec caps 100).
- **352.3 (#630) — `application/a2a+json` + `includeArtifacts`.** The REST §11
  responses now carry the spec media type `application/a2a+json`;
  `includeArtifacts` is accepted on both `tasks/get` and `tasks/list` (the field
  is omitted when false, per the spec's omit-when-default rule).
- **352.4 (#631) — `statusTimestampAfter` filter.** `list_a2a_tasks` gains an
  `updated_after: Option<DateTime<Utc>>` push-down (both backends) so an agent
  can poll "what changed since I last looked". A gate-task's status timestamp is
  when the gate became pending (`gate.created_at`), so gates filter the same way.
  A malformed value is a `-32602` / 400, not a silent pass.

## Decisions

- **Synthesize gates as tasks; do not overlay real tasks.** The first cut
  overlaid real `maidan_a2a_tasks` rows for pending gates. That would have broken
  A2A task-state monotonicity (a task, once sent, is terminal or working — it
  cannot regress to `input-required`) and risked the a2a-tck. The synthetic
  read-time view is spec-honest: a gate *is* an input-required task from the
  protocol's point of view, and it disappears from the list the instant the gate
  resolves.
- **Defer 352.5 (real `nextPageToken` keyset paging).** The last H12 line-item —
  paginate beyond `pageSize` — was deferred as a **logged follow-up**, not built.
  Correct paging needs the per-channel RBAC filter pushed **into** the store
  query: the current post-fetch RBAC filter drops rows, so a keyset/offset cursor
  cannot reliably fill a page or know it is the last one. That is a real store
  refactor for a **rare** edge (it only bites a workspace with **>100** tasks,
  which never happens for H12's actual use — a handful of pending gates, well
  under `pageSize`). The always-`""` token is already conformant for the ≤100
  common case. Gold-plating it now was the wrong trade; it is recorded in Open
  Work with the RBAC-in-query prerequisite.

## Surprises

- **The overlay dead-end cost a rebuild.** 352.1's first design (overlay + a
  `pending_approval_gate_thread_ids` store method) was backed out entirely once
  the monotonicity conflict surfaced — the synthetic `gate_as_task` replaced it.
- **`A2aMessage` has no `Part` enum.** The real shape is
  `A2aMessage { role, parts: Vec<TextPart>, metadata }`; `gate_as_task` builds
  the message from that, not a hypothetical `Part`.
- **The SQLite `Z`-vs-`+00:00` datetime trap, again.** 352.4's timestamp filter
  wraps **both** sides in `datetime(...)` so the stored `...Z`-millis form and
  chrono's `...+00:00` compare as the same UTC instant (the occupancy-clocks
  lesson carried over first-try).
- **The gate-task e2e needs auth enabled.** Under bypass auth,
  `auth.workspace_id` is the nil workspace, so the synthesized gate-tasks came up
  empty; the H12 e2e mints a real bearer token so the workspace scoping is real.

## Test evidence

- `a2a_protocol_e2e::a2a_pending_gate_surfaces_as_input_required_task` — the
  end-to-end H12 assertion: a pending gate shows in `tasks/get` and
  `tasks/list` as `input-required`, the `status` filter selects it, `pageSize`
  clamps at 100, the content type is `application/a2a+json`,
  `statusTimestampAfter` far-future = empty / far-past = all / malformed = 400,
  and the gate-task disappears the moment the gate resolves.
- `a2a_store` (both backends) — `list_a2a_tasks` with and without the
  `updated_after` filter.
- `a2a_grpc_e2e` green (the gRPC `tasks/list` path shares the dispatch).
- Every sub-PR: fmt + strict clippy (`-D clippy::unwrap_used`) + both-backend
  store tests, admin-merged green across all required checks.

## Forward look

**The HITL list is complete for discovery.** An external A2A agent can now find
a pending held gate via `tasks/list?status=input-required` (or
`?statusTimestampAfter=…`) over `application/a2a+json`. The one deferred
line-item — 352.5 real `nextPageToken` paging — is logged in Open Work behind
its RBAC-in-query prerequisite.

Next-ranked (Wave 1 #4): **N8 / B1 + N7** — session chrome
(running/idle/needs-input/needs-approval/done) + a capability card, and flagship
buttons in the vanilla `/ui` (WCAG AA, keyboard, `lang`). No SPA.

## Acknowledgements

Built as a four-PR stack (#628 → #631) on top of the Cluster 350 held gate,
each rebased onto `main` as its parent merged, validated locally under the
GitHub-Actions cadence.
