### A claimant's background renewer dies with its session

- **Fixed:** the runbook's background lease renewer outlived a session that died mid-gate (a spend limit, 2026-10-22) and held ENG-5650's claim for seven hours with no worker. It now checks its parent shell each pass (`kill -0`), stops at the step's longest expected run, and is killed by the shell's exit trap.
