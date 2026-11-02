### A named review head must be checkable

- **Changed:** `submit_review` (REST and MCP) now refuses a `head_sha` with a conflict when the change has no tracked pull request head to check it against, on both backends, instead of storing a head nobody verified. Omitting `head_sha` still works (#1744).
