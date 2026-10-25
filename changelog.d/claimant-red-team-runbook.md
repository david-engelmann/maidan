### A red-team runbook, and failed attempts left on the thread

- **Added:** `examples/claude-code/maidan-claimant/red-team.md`, a harness-agnostic runbook for review threads: check out the exact head, scope a delta or merge delta, probe adversarially, make the minimal wrong change to show a test pins its behaviour, and deliver a VERDICT, HEAD, BLOCKING, NON-BLOCKING and TESTS RUN report as a pull-request comment.
- **Changed:** the claimant runbook asks a seat to post each failed attempt on the thread as `FAIL: <hypothesis> | <evidence>` before it delivers or releases, so the next claimant does not repeat it.
