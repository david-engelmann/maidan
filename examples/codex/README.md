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
   (`wait_ms`); Codex's default would cut it off first. It approves the
   Maidan tools up front, because nobody is there to answer a prompt (see
   below), and gives the sandbox network access. Change the URL if Maidan is
   not on this machine (use https then).

## Run one task

From the checkout the task is for:

```sh
export MAIDAN_URL=http://127.0.0.1:8080
export MAIDAN_TOKEN="$(cat ~/.config/maidan/seat-token)"
codex exec -m gpt-5.6-sol --sandbox workspace-write --ephemeral \
  "$(cat /path/to/maidan/examples/codex/prompt.md)" < /dev/null
```

- **Model.** Name it with `-m`. A ChatGPT login serves only the models in
  `~/.codex/models_cache.json`, and refuses others with a 400 ("not supported
  when using Codex with a ChatGPT account") even when `config.toml` names
  them as the default. `gpt-5.6-sol` is the one this recipe was run with; an
  API-key login can name any model the key serves. `< /dev/null` keeps
  `codex exec` from waiting on stdin when it runs from a script.
- **Approvals.** Codex asks before any MCP tool annotated destructive, and
  under `exec` nobody answers, so the call is refused. `report_usage` is one
  (over budget it ends the run's claim). `default_tools_approval_mode =
  "approve"` in [`config.toml`](config.toml) lets the session call the Maidan
  tools; what the session may do is still bounded by its seat token and by
  Maidan's gates, not by the client.
- **Sandbox.** `workspace-write` lets Codex edit the checkout and run git,
  the gate and the helper. The runbook also makes a few REST calls with curl
  (the task's attributes, the result). Those need network access from the
  sandbox: allow it for this workspace
  (`[sandbox_workspace_write] network_access = true`, in
  [`config.toml`](config.toml)), or run with a sandbox that has it. MCP
  calls go through Codex itself and are not sandboxed.
- **One task per run.** [`prompt.md`](prompt.md) has Codex take one task and
  stop. To keep a seat on the queue, run it again when it ends; each run
  waits for work in the claim itself, so there is no sleep between runs.
- **Usage.** Codex names itself in `report_usage` with
  `evidence: {"provider": "openai", "harness": "codex"}`, which the swarm
  metrics slice on.
