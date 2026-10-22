# Codex CLI as a Maidan claimant

`codex exec` works one Maidan task per run with the shared
[claimant runbook](../claude-code/maidan-claimant/runbook.md): it claims a
task routed to sessions, does it in the checkout, runs the repository's
pre-push gate, hands back a `pi.change.result/1` so Maidan opens the draft
pull request, reports usage and releases the claim. Nothing here is copied
from the runbook; the run prompt points Codex at it.

## Set it up

1. **Mint a seat** (once per person and machine; needs a `token:admin`
   token). Runbook step 0 has the command. It writes the token to
   `~/.config/maidan/seat-token` without printing it.
2. **Install the runbook and the helper** where the prompt expects them:

   ```sh
   mkdir -p ~/.codex/maidan-claimant
   cp examples/claude-code/maidan-claimant/runbook.md \
      examples/claude-code/maidan-claimant/change_result.py ~/.codex/maidan-claimant/
   ```

   (from a checkout of Maidan). That directory is the runbook's
   `$CLAIMANT_DIR`.
3. **Connect.** Merge [`config.toml`](config.toml) into
   `~/.codex/config.toml`. It names the environment variable the token is
   read from (`bearer_token_env_var`), never the token. It also raises the
   MCP tool timeout to 330 s, because a claim waits up to 300 s for work
   (`wait_ms`); Codex's default would cut it off first. Change the URL if
   Maidan is not on this machine (use https then).

## Run one task

From the checkout the task is for:

```sh
export MAIDAN_URL=http://127.0.0.1:8080
export MAIDAN_TOKEN="$(cat ~/.config/maidan/seat-token)"
codex exec --sandbox workspace-write "$(cat /path/to/maidan/examples/codex/prompt.md)"
```

- **Sandbox.** `workspace-write` lets Codex edit the checkout and run git,
  the gate and the helper. The runbook also makes a few REST calls with curl
  (the task's attributes, the result). Those need network access from the
  sandbox: allow it for this workspace
  (`[sandbox_workspace_write] network_access = true` in `config.toml`), or
  run with a sandbox that has it. MCP calls go through Codex itself and
  are not sandboxed.
- **One task per run.** [`prompt.md`](prompt.md) has Codex take one task and
  stop. To keep a seat on the queue, run it again when it ends; each run
  waits for work in the claim itself, so there is no sleep between runs.
- **Usage.** Codex names itself in `report_usage` with
  `evidence: {"provider": "openai", "harness": "codex"}`, which the swarm
  metrics slice on.
