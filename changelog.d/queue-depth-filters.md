### Workspace queue depth takes the claim filters

- **Added:** `GET /workspaces/{wid}/queue-depth` and the `get_workspace_queue_depth` tool take the `repo` and `engine` filters `claim_next_workspace_thread` takes, so a pool of workers serving one repository or engine is sized by the work it can actually claim.
