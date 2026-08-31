# Cluster 342.0 retro — surface the flagship context features to integrators (audit P2)

> Tag **`v342.0.0`**. Phase XXIV (post-gate hardening). **Cluster 11 of the post-flagship audit
> program.** Docs-only. No new gate tag.

## What shipped

`Integration.md` — the single integrator entry point — described the context pack as "messages,
edits, references, artifacts, FSM history (paginated)" and stopped there. The features that make
Maidan's context *the* differentiator were invisible to anyone reading the integrator guide: a
promoter couldn't see them, and an integrator wouldn't know to use them.

- A new **"Fidelity & context"** subsection in `Integration.md` documents, with the exact wire
  surface (verified against `dto.rs` / `app.rs` / the MCP catalog):
  - **Glossary grounding** — `include_glossary` on the pack + the `/workspaces/:wid/glossary`
    term CRUD.
  - **As-of replay (time travel)** — `as_of=<event_log_id>` reconstructs the thread as it stood
    at a point in the immutable log.
  - **Context snapshot** — `POST /threads/:id/context/snapshot` freezes a pack into the
    content-addressed store (tamper-evident, deduped).
  - **Lean edits** — `include_edits` (default lean) as the largest token lever.
  - **Seed / re-ask** — `POST /messages/:id/seed` spins a fresh thread linked back by a
    `seeded_from` edge.
  - **Tool-call transcript** — `GET /threads/:id/tool-transcript`.
  - An MCP-parity line (all six have MCP tool/param equivalents — verified present in
    `contracts/mcp-tool-names.json` + `catalog.rs`).
- Folded a tool-count miss from Cluster 341: `Protocols.md` line 54 still said "tool count is
  **78**" → **85**.

## Surprises / decisions

- **The gap was discoverability, not capability.** Every feature already shipped and is tested;
  they were simply absent from the one doc an integrator is told to read. This is the cheapest
  possible "compelling offering" win — surfacing what already exists.
- **`as_of` is an event-log id, not a timestamp.** Documented as such (`Option<i64>` in
  `ThreadContextQuery`), so an integrator passes the right thing.
- **Accuracy-first.** Every claim (routes, params, capabilities, MCP parity) was checked against
  code before writing, per the P2 discipline.

## Test evidence

Docs-only. `mdbook build` + linkcheck clean.

## Forward look

Remaining audit items: **P1.5** (egress wire-path tests + LSN replica CI) and the P2 code-side
items (projector link-management surface, notification-router fan-out, Store trait split,
unbounded `list_threads` pagination, MCP `post_message` slash-dispatch decision).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
