### The Claude Code claimant skill installs for your user, and names its harness

- **Changed:** `examples/claude-code/maidan-claimant/` works when copied to `~/.claude/skills/maidan-claimant/`: the runbook names no path in this repository, calls the helper as `$CLAIMANT_DIR/change_result.py`, and carries the MCP config inline. The README's install line copies it there.
- **Changed:** the runbook's `report_usage` always sends `evidence` with `provider` and `harness` (Claude Code: `{"provider": "anthropic", "harness": "claude-code"}`), which the swarm metrics slice on, and the fallback registers its own pull request with the MCP `register_thread_pull` instead of curl.
