# The Cursor CLI as a Maidan claimant

`cursor-agent`, Cursor's command-line agent, works one Maidan task per run
with the same rule and runbook as the [Cursor editor](../cursor/): it claims
a task routed to sessions, does it in the checkout, runs the repository's
pre-push gate, hands back a `pi.change.result/1` so Maidan opens the draft
pull request, reports usage and releases the claim. Nothing here is copied
from the runbook; the run prompt points the agent at it.

## Set it up

1. **Mint a seat and install the rule, the runbook and the helper** exactly
   as the [Cursor setup](../cursor/README.md) says, steps 1 and 2. The CLI
   reads the same `.cursor/rules/` and the same `.cursor/mcp.json` as the
   editor, so the runbook's `$CLAIMANT_DIR` is `.cursor/maidan-claimant`.
2. **Connect.** Merge [`../cursor/mcp.json`](../cursor/mcp.json) into
   `.cursor/mcp.json` (the token stays an `${env:MAIDAN_TOKEN}` placeholder),
   then check the CLI sees the server:

   ```sh
   export MAIDAN_TOKEN="$(cat ~/.config/maidan/seat-token)"
   cursor-agent mcp list
   ```

   The first run may ask you to approve the `maidan` server; approve it once
   in an interactive `cursor-agent` session, or use your version's approval
   flag (`cursor-agent --help`).

## Run one task

```sh
export MAIDAN_URL=http://127.0.0.1:8080     # https for anything but this machine
export MAIDAN_TOKEN="$(cat ~/.config/maidan/seat-token)"
cursor-agent -p "$(cat /path/to/maidan/examples/cursor-agent/prompt.md)"
```

`-p` runs without a person at the keyboard, so the agent can only do what
the CLI's permissions allow. The work needs a shell (git, the gate, the
helper) and file edits. Give it those in the repository's CLI permissions,
or pass your version's flag that allows commands (`--force` in current
releases); read what it allows before using it.

[`prompt.md`](prompt.md) has the agent take **one** task and stop. To keep a
seat on the queue, run it again when it ends; each run waits for work in the
claim itself (`wait_ms`), so there is no sleep between runs.

The agent names itself in `report_usage` with
`evidence: {"provider": "<the model's vendor>", "harness": "cursor"}`. The
provider is the vendor of the model the run used (`anthropic`, `openai`,
`xai`, …); the swarm metrics slice on both fields.
