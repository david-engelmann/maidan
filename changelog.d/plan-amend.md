### Amend a submitted plan (#1465)

- **Added:** `POST /threads/{id}/plan/amend` and the MCP `amend_plan` let the planner holding a plan's claim cancel unclaimed children, add children, replace one (cancel and add the same key, so its dependents wait for the new child) and rewire unclaimed children's dependencies, all in one transaction. A claimed or finished child, or a fourth amendment, is a 409 and nothing changes. Migration 0198 adds the amendment count to `maidan_thread_plans`.
