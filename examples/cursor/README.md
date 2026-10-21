# Cursor as a Maidan claimant

[`maidan-claimant.mdc`](maidan-claimant.mdc) is a Cursor project rule that
lets an Agent session take Maidan work: claim a task routed to sessions, do it
in your checkout, run the repository's pre-push gate, and hand the change back
so Maidan commits it and opens the draft pull request. The procedure is the
same runbook the [Claude Code skill](../claude-code/) uses, so the two stay
one text.

## Set it up

1. **Mint a seat** (once per person and machine; needs a `token:admin`
   token). Step 0 of the [runbook](../claude-code/maidan-claimant/runbook.md)
   has the command: it calls `POST /workspaces/{wid}/agents` and writes the
   new token to `~/.config/maidan/seat-token` without printing it.
2. **Install the rule and the runbook** in the repository you work in:

   ```sh
   mkdir -p .cursor/rules .cursor/maidan-claimant
   cp /path/to/maidan/examples/cursor/maidan-claimant.mdc .cursor/rules/
   cp /path/to/maidan/examples/claude-code/maidan-claimant/runbook.md \
      /path/to/maidan/examples/claude-code/maidan-claimant/change_result.py \
      .cursor/maidan-claimant/
   ```

   The rule is agent-requested (`alwaysApply: false`): the Agent loads it
   when you ask for Maidan work.
3. **Connect.** Merge [`mcp.json`](mcp.json) into `.cursor/mcp.json`. It
   names `${env:MAIDAN_TOKEN}`, never the token itself, so the file is safe to
   commit; change the URL if Maidan is not on this machine (use https then).
   Start Cursor with the token read from its file, so the variable is in its
   environment:

   ```sh
   MAIDAN_TOKEN="$(cat ~/.config/maidan/seat-token)" cursor .
   ```

4. **Ask the Agent for work:** "take the next Maidan task".

A claim waits up to five minutes for work (`wait_ms`). If Cursor times the
tool call out sooner, lower `wait_ms` below its timeout; an empty answer means
call again, never sleep and poll.

What the rule holds to is the same as the skill's: the token never appears,
Maidan makes the commit (with the runbook's fallback for binary or oversized
changes), follow-ups are new commits and never a force-push, and the merge
gate is unchanged.
