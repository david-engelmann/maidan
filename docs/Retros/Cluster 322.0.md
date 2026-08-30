# Cluster 322.0 retro — glossary REST + MCP

> Tag **`v322.0.0`**. Phase XXIV (post-gate hardening). **Cluster 4 of the fidelity +
> context flagship arc.** No new gate tag.

## What shipped

The 321 glossary foundation, surfaced over both wire surfaces. Agents can now define,
look up, and list a workspace's canonical `term -> definition`.

- **REST** (`routes/glossary.rs`) — `PUT /workspaces/:wid/glossary/:term` (define/upsert,
  `workspace:write`), `GET /workspaces/:wid/glossary` (list, `workspace:read`), `GET
  /workspaces/:wid/glossary/:term` (one, `404` if undefined), `DELETE
  /workspaces/:wid/glossary/:term` (`204`/`404`, `workspace:write`). Workspace-scoped:
  `cap` → `ensure_workspace` → `get_workspace` (existence). `created_by` is the acting
  member (`auth.member_id`).
- **MCP** (`tools/glossary.rs`) — `set_glossary_term` / `get_glossary_term` /
  `list_glossary_terms` over the caller's `auth.workspace_id`. `set` is `workspace:write`
  (+ `created_by`), reads are `workspace:read`. `delete` stays REST-only (the 220/229
  precedent — MCP gets the agent-loop verbs, housekeeping stays REST). Standard 5-place
  wiring + both sorted contracts (82 tools now).

## Surprises / decisions

- **Term is the path segment for get/delete/put; the body carries only `definition` +
  `aliases`.** The term is the resource id — clean REST, and the same `:skill` by-value
  path-param precedent (Cluster 232). Terms with a `/` aren't supported (documented);
  spaces URL-encode.
- **`created_by` FK forces auth-enabled tests.** The nil-member bypass would FK-fail on
  `set` (memory-noted since 228/232), so both the REST e2e and the MCP inline test use a
  real minted token / `AuthContext::from_session`.
- **Redefining resets `aliases` when omitted.** `PUT` is a full upsert of the body, not a
  patch — omitting `aliases` clears them (the e2e pins this). A term is small; a
  merge-patch semantics would be surprising for a definition.
- **No context-pack fold here.** That is the higher-value, more invasive change (it touches
  the `ThreadContext`/`WorkspaceContext` DTOs and every context consumer) — it earns its
  own cluster (**323**) rather than bloating this one. This keeps each cluster testable and
  the diff reviewable.

## Test evidence

`glossary_rest_e2e` (define/list/get/404/upsert-no-dup/aliases-reset/empty-400/delete-
conditional, auth-enabled + minted token); `glossary_tools_set_get_and_list` MCP inline
test (set/get/null-on-missing/list, real-member session); MCP contract-sync + capability
matrix (deny-without-cap + pass-with-cap) + `http_capability_matrix_e2e` (the new PUT/DELETE
routes deny without the cap, with a `{term}` substitution + PUT body clause) + `openapi_e2e`
bijection green. fmt + strict clippy + `--all-targets` + bootstrap-strip clean; mdbook
linkcheck green.

## Forward look

**323** folds the glossary into the context pack — a thread's `GET …/context` (REST + MCP)
carries the workspace's definitions, so an agent's context is grounded in the shared
vocabulary without a second call. Then the rest of the arc: confidence/conventions → as-of
context replay → seed-from-message → context snapshot artifact.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the fidelity + context
flagship arc ([[Open Work]]).
