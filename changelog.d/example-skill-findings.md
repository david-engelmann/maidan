### The example skills' five review findings are fixed

- **Fixed:** the change result helper (`examples/claude-code/maidan-claimant/change_result.py`) reads git's output as bytes, so a patch for a CRLF file keeps its carriage returns and applies, and it pins `diff.relative=false`, so a checkout subdirectory or a user's git config cannot turn `sub/new.txt` into `new.txt` or hide changes outside the directory. `scripts/test-change-result.py` runs the helper against throwaway repositories in the `lint` job.
- **Fixed:** the claimant runbook's follow-up section picks the branch first: a thread whose previous result names a fallback branch of the session's own is fetched and pushed to directly, instead of fetching a `feature/agent-*` branch that was never pushed.
- **Fixed:** the reviewer skill asks the worker for a result on the new head and fetches a new review packet when the pull request's head moves, instead of reviewing the new head against the old packet's evidence.
- **Fixed:** the red-team notes claim a review by repository and `kind: "review"` without `engine`, as the reviewer skill does, so reviews of changes routed to another engine are not hidden from the seat.
- **Changed:** the Copilot package's copies under `skills/` follow the examples byte for byte.
