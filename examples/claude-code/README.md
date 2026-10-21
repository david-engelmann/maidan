# Claude Code as a Maidan claimant

[`maidan-claimant/`](maidan-claimant/) is a Claude Code skill that lets an
interactive session take Maidan work the way an unattended worker does: it
claims a task routed to sessions, does it in your checkout, runs the
repository's pre-push gate, and hands the change back so Maidan commits it
and opens the draft pull request. Then it reports usage, moves the thread to
review and releases the claim.

| File | What it is |
|---|---|
| [`maidan-claimant/SKILL.md`](maidan-claimant/SKILL.md) | The skill: when to use it and the loop in short |
| [`maidan-claimant/runbook.md`](maidan-claimant/runbook.md) | The procedure, with every call's arguments. The [Cursor rule](../cursor/) uses the same text |
| [`maidan-claimant/change_result.py`](maidan-claimant/change_result.py) | Builds the `pi.change.result/1` envelope from git and checks it against what the change flow accepts. Standard library only, no network |
| [`mcp.json`](mcp.json) | The `.mcp.json` entry for Maidan |

## Set it up

1. **Mint a seat** (once per person and machine; needs a `token:admin`
   token). Runbook step 0 has the command: it calls
   `POST /workspaces/{wid}/agents` and writes the new token to
   `~/.config/maidan/seat-token` without printing it.
2. **Install the skill** in the repository you work in (or in
   `~/.claude/skills/` for every repository):

   ```sh
   mkdir -p .claude/skills
   cp -R /path/to/maidan/examples/claude-code/maidan-claimant .claude/skills/
   ```

3. **Connect.** Merge [`mcp.json`](mcp.json) into the repository's
   `.mcp.json`. It names `MAIDAN_URL` and `MAIDAN_TOKEN`, never the token
   itself, so the file is safe to commit. Start Claude Code with the token
   read from the file:

   ```sh
   export MAIDAN_URL=http://127.0.0.1:8080      # https for anything but this machine
   MAIDAN_TOKEN="$(cat ~/.config/maidan/seat-token)" claude
   ```

4. **Ask for work:** "take the next Maidan task". The skill claims with
   `engine: "session"`, so it is handed only work a producer routed to
   sessions, and with this checkout's `repo`, so only work for this
   repository.

A claim waits up to five minutes for work (`wait_ms`) and is called again
when it comes back empty. If Claude Code times the call out first, raise
`MCP_TOOL_TIMEOUT` (milliseconds) or lower `wait_ms`.

## What the skill holds to

- **The token never appears.** It is read from its file into the
  environment; it is never echoed, pasted into the chat, or written to the
  repository.
- **Maidan makes the commit.** The session sends a base commit and a text
  diff; Maidan applies it on a `feature/agent-*` branch and opens a draft.
  A change the flow cannot commit (binary files, symlinks, submodules, a
  result over the request limit) goes through the fallback in the runbook:
  the session pushes its own branch, opens the draft itself, and registers
  it on the thread with `PUT /threads/{id}/pull`.
- **Follow-ups are new commits.** Never a force-push.
- **The merge gate is unchanged.** The session never marks a pull request
  ready, approves, merges, labels or requests reviewers.
- **Release on every exit.** A finished thread goes to review first, so it
  is not handed out again.
