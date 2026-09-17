# R-01 — Audit method

## Scope decision

The audit was scoped by an explicit instruction: **evaluate off `main`, not the most recent tag**, because tags lag behind. All findings reference `main` @ `e232c73` (reported as Cluster 399.2). Line numbers will drift; file paths are the stable identifiers used throughout this corpus.

## How the work was divided

Four parallel investigation tracks, each reading a different slice of the repo:

1. **Docs & examples** — README, `docs/` (~50 files), `examples/`, SDK READMEs, wording/voice.
2. **Rust architecture** — the 14-crate workspace: contracts, auth, store, server, CLI; enforcement structure; migration discipline.
3. **Contracts, auth & CI** — event kinds, capability map, MCP tool surface, `maidan-auth`, CI workflows, test depth.
4. **Ops & release hygiene** — Docker/Compose, Helm, k8s, Makefile, tagging, CHANGELOG, version pins.

Each track read files directly and cross-checked claims across files (e.g., README pins vs CHANGELOG vs tags; SDK README snippets vs the `ClaimedThread` contract; example auth assumptions vs the Cluster 313 auth-on change).

## What was *not* done

- Nothing was executed against a live server. Behavioral claims (e.g., "`make smoke` starts only Postgres") are inferred from reading the Makefile and compose files, not from running them. The building agent should verify before treating these as confirmed.
- The MCP tool surface (177 tools) was counted and spot-checked, not exhaustively reviewed tool by tool.
- Frontend or docs-site rendering was not inspected beyond noting the linkcheck gate in CI.

## Second verification pass (same day, 399.3)

The highest-stakes claims were re-verified against a fresh clone at `207047c` (Cluster 399.3, one cluster after the audit): F-06's three exemplar routes (confirmed at handler level with file:line), F-10's claim-shape mismatch (confirmed: `#[serde(flatten)]` on `ClaimedThread.thread` means no `"thread"` key in JSON; the Python README's `res.get("thread")` returns `None`), F-32's `make smoke` (confirmed: `compose-up` runs `docker compose up -d` with no profile, and only `postgres` has no profile gate), F-34's 191 `include_str!` entries (exact count confirmed), and the CLI's DB-direct architecture (confirmed: `sqlx` pools, `maidan-store` dep, no HTTP client). Three discrepancies were corrected: the audit's "~43" enforcement call sites was an undercount (actual ~260 `cap(&auth, …)` lines), `.env.example` exists but is unlinked from the docs, and app services (not infra) are the ones missing compose healthchecks.

## How recommendations were formed

For each finding, the track asked three questions: (1) what does the file actually say/do, (2) what does a *different* file in the same repo say/do about the same thing, (3) what would an automated agent building on this surface experience in the first 15 minutes. Findings where (2) showed a contradiction became P0/P1; findings where only (3) showed friction became P2 polish.

The severity scale (P0/P1/P2) is defined in `README.md`. P0 was deliberately kept small: only findings that block safe adoption or cause silently wrong behavior.
