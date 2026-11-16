### A queue count of what claim-next would hand you

- **Added:** `GET /workspaces/{wid}/queue-depth?claimable_by=me` and `get_workspace_queue_depth` with `claimable_by: "me"` count only the threads claim-next would hand the caller. A review of the caller's own model family's change, a thread it declined as its family's, one that needs a skill it lacks, and every thread while it is frozen are left out. A pool that reads each seat's count with that seat's token wakes only the seats with work they would be handed. Without it, the counts are unchanged.
