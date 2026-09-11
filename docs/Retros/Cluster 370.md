# Cluster 370 retro — Wave 2 #18: a recipe / thread-type (G8 + W5 + G-dev-9)

Wave 2 #18 gives the room a **recipe**: a reusable thread-type blueprint (the
Goose-recipe *shape* — params, a definition of done, a retry policy, and inline
child sub-tasks that form a DAG). Instantiating one builds a parent thread + its
DAG children + attaches required skills, freezing the recipe bytes into a run
snapshot (copy-on-fire). A schedule can seed a run, skipping when the prior run is
still in flight. **Not a recipe VM** — a blueprint the room instantiates, not an
execution engine.

## What shipped

- **370.1 (#735) — the store foundation.** `maidan_recipes` (pg 0073 / sqlite
  0072) + the `RecipeSpec` blueprint types in maidan-types + a pure
  `RecipeSpec::validate` (unique non-empty child keys, deps reference real
  siblings, no self-dep, **acyclic child DAG** via Kahn's algorithm) +
  `validate_params` (required present) + the `RecipeStore` CRUD, both backends.
  Zero-blast-radius (the 159/217/226 pattern).
- **370.2 (#736) — instantiation.** `maidan_recipe_runs` (pg 0074 / sqlite 0073)
  + `instantiate_recipe` — in one transaction: parent thread + a child per
  `spec.children`, wire the DAG (each child depends on its `depends_on` siblings;
  the parent depends on every child so it lands last), attach skills, and write
  the copy-on-fire snapshot. Reuses the `create_thread_with_event` shape
  (`events::append_in_tx` per thread) + the existing dep/skill tables.
- **370.3 (#737) — REST.** create / list / get / delete + `instantiate` (→ a
  `RecipeRun`, publishing the `ThreadCreated` events). Full new-route preflight.
- **370.4 — MCP.** `create_recipe` / `list_recipes` / `instantiate_recipe`, the
  agent-native twins.
- **370.5 — scheduler seeding + `ScheduleSkipped`.** `task_schedules.recipe_id`
  (pg 0075 / sqlite 0074); the sweeper instantiates the recipe on each firing
  (copy-on-fire) instead of a bare thread, and emits `ScheduleSkipped` (a new
  non-federatable EventKind) when the prior run is still in flight.

## Decisions

- **A recipe is a blueprint, not a VM.** `retry` and `definition_of_done` ride
  the frozen snapshot but are *not* enforced (enforcement is #22 reviewers / #25
  soundcheck). Children are inline definitions, not references to other recipe
  rows — true sub-recipes (recursive instantiation) would be VM-shaped, so they're
  a follow-up. This keeps instantiation a bounded, single-transaction build.
- **A task is a thread (again).** Instantiation reuses the thread FSM +
  `thread_dependencies` (Cluster 217) + `thread_required_skills` (231), so a
  recipe run is claimable, DAG-ordered, and skill-routed for free — no new task
  entity.
- **Validate the DAG at rest, trust it at instantiation.** `RecipeSpec::validate`
  (pure, at create) proves the child graph acyclic, so `instantiate_recipe` wires
  dependencies without re-running the cycle guard; the parent-depends-on-children
  edges add no cycle (a fresh node with no dependents).
- **Copy-on-fire is the reproducibility record.** `recipe_runs.spec_snapshot`
  freezes the recipe bytes at fire time, so editing the recipe later never
  changes what a past run was.
- **`ScheduleSkipped`, not silent dedup.** A skipped firing emits an observable
  event (non-federatable — a local scheduler decision), so an operator sees the
  run didn't pile up rather than wondering where it went.

## Surprises

- **`+` at a doc-line start is a markdown bullet.** `clippy::doc_lazy_continuation`
  red the scheduler doc comment where a wrapped line began "`+ DAG children`" —
  reworded to "a parent thread with DAG children" (same class as the Cluster-358
  gotcha).
- **The scheduler fires recipes param-less.** A schedule carries no params, so a
  recipe with required params can't be scheduled (instantiation would 400).
  Schedule-level params are a logged follow-up.

## Test evidence

- maidan-types: `RecipeSpec` validation unit tests (the DAG + param rules).
- Store: `recipes` (both backends) — CRUD + JSON round-trip + instantiation (3
  ThreadCreated, the frozen snapshot, parent→children + review→build edges, the
  attached skill, run round-trips, missing-param rejected).
- Server: `recipe_rest_e2e` (auth-enabled), `scheduler_e2e` (recipe seeding +
  ScheduleSkipped via a bus subscriber), the bijection / matrix / openapi↔map
  contracts.
- MCP: `recipe_tools_create_list_and_instantiate` + the catalog / capability-map
  contract-sync guards.

## Forward look

**Wave 2 #18 is complete.** Deferred (follow-ups): sub-recipes as references to
other recipes (recursive instantiation); param templating in child titles;
schedule-level params; full JSON-Schema param validation (a validator dep);
webhook/slash seeding (the schedule path demonstrates seeding); DoD/retry
*enforcement*. **Next: Wave 2 #19** (G19 + T3 — secret-ref: the log holds an id,
the store holds the value, Pi fetches at exec; a SecretBroker substitutes on
egress with a host allowlist).

## Acknowledgements

Five impl PRs (#735 store → #736 instantiate → #737 REST → 370.4 MCP → 370.5
scheduler) plus this retro, on the foundation-then-wire (217/226) + new-route
preflight + EventKind-drill patterns.
