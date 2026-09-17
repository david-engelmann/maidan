# INIT-04 — Scheduler discoverability

**Findings:** F-13 (P1)

## Problem statement

Task schedules are a real, shipped subsystem: introduced in Clusters 228–229, with a real sweeper, REST endpoints, and MCP tools (`create_task_schedule` and related). Yet they are **absent from `docs/Integration.md`, from every example, and from all four SDK READMEs**. The only place they surface is the changelog.

## Why it matters to an automation-layer consumer

For the stated audience — people building automated agent development workflows — scheduled/recurring agent work is arguably the headline capability. A builder evaluating Maidan against alternatives will not find it during evaluation, because evaluation happens in `Integration.md` and the examples, not in 400 clusters of changelog. This is a discoverability gap for a differentiating feature, not a missing feature.

## Advisory recommendation

- Document the task-schedule lifecycle where builders look: a section in `docs/Integration.md` (create → list → trigger behavior → cancellation), one example (e.g., extend `lease_demo` or a small standalone script), and a mention in each SDK README's capability list.
- Cover the operational semantics builders will ask about: what happens on missed runs, what the sweeper's granularity is, how schedules interact with leases/claims, and how to observe schedule executions.
- If there are known limitations (e.g., no catch-up, single-node sweeper), state them — the repo's docs are strongest when honest (`docs/Claims.md` is the model).

## Open questions for the building agent

- Is the scheduler considered stable API or still evolving? That determines whether it gets Integration.md treatment or a clearly-marked preview section.
- Should SDKs get schedule methods, or is HTTP/MCP the intended surface (ties into INIT-03's provisioning question)?
- Are there multi-node / HA semantics for the sweeper that need documenting before promoting it?

## Signals of resolution

- A builder can learn task schedules exist from `Integration.md` alone.
- One runnable example exercises create → observe → delete.
- SDK READMEs mention schedule support (or its deliberate absence).
