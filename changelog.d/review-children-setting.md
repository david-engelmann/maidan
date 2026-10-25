### Workspace review-children setting

- **Added:** a per-workspace `review_children` setting (migration 0208). `GET` and `PUT /workspaces/{id}/review-children` read and set it: `spawn` (the default, today's behaviour) or `off`. Setting it takes `token:admin` and is audited as `workspace.review_children.set`. With `off`, a change entering review spawns no second-family `kind: review` child, and review stays with the pull request's own reviewers.
