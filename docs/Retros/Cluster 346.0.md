# Cluster 346.0 retro — projector link-management REST surface (audit P2)

> Tag **`v346.0.0`**. Phase XXIV (post-gate hardening). **Cluster 15 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The Slack (307–309) and GitHub (310–312) projectors shipped ingress, egress, and a store link
table — but **no route or tool ever created a link**. The link table could never be populated, so
the egress (`route_message_to_slack` / `route_message_to_github`, which read
`get_*_link_by_thread`) could never fire. The projectors were a launch-narrative feature that
could not be turned on. This closes that gap:

- **Slack:** `POST` / `GET /workspaces/:wid/slack-links`, `DELETE /workspaces/:wid/slack-links/:slack_channel_id`.
- **GitHub:** `POST` / `GET /workspaces/:wid/github-links`, `DELETE /workspaces/:wid/github-links?repo=…&issue_number=…`.

`POST`/`DELETE` are `workspace:write`, `GET` is `workspace:read`.

## Surprises / decisions

- **The link's `channel_id`/`workspace_id` are derived, not trusted.** The create handler resolves
  the thread with `maidan_auth::authorize_thread` (Cluster 339) and takes the scope's channel +
  workspace, so a client can't create a link whose channel disagrees with its thread. The caller
  supplies only the external id, the thread, and the member relayed messages are attributed to.
- **GitHub unlink is a query pair, not a path param.** `repo` is `owner/name` (contains a slash),
  so `DELETE …/github-links?repo=…&issue_number=…`. The query fields are `Option` **only** so a
  request that omits them fails the capability check (403) rather than query extraction (400) —
  which keeps the capability-matrix test's cap-less probe meaningful; the handler still requires
  both and returns `400` when they're missing.
- **Unlink is workspace-scoped.** A `DELETE` only removes a link that belongs to the path
  workspace (checked via the reverse lookup first), so one tenant can't unlink another's link by
  guessing a Slack channel id.

## Test evidence

`projector_links_e2e` (create a Slack + a GitHub link, assert the derived `channel_id`/`workspace_id`,
assert the **egress reverse-lookup** now resolves the link, list it, unlink it — a second unlink
`404`s, a query-less GitHub unlink `400`s). `openapi_e2e` bijection + `http_capability_matrix_e2e`
(deny-without-cap, incl. the new POST bodies + `{slack_channel_id}` substitution) green. fmt +
strict clippy + `--all-targets` + bootstrap-strip clean (handlers placed before the module's
`#[cfg(test)]` to satisfy `items_after_test_module`).

## Forward look

Remaining audit items: **P1.5** (egress wire-path tests + LSN replica CI — now higher-value since
the projector egress is operable); P2 code-side — the notification batch insert (Cluster-344
follow-up) and the Store trait split. MCP projector-link tools are an optional follow-up (REST is
the operable surface).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
