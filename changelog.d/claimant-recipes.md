### The Cursor CLI, Codex and any OpenRouter model can claim Maidan work

- **Added:** claimant recipes for the command line, each one task per run on the shared claimant runbook: `examples/cursor-agent/` (`cursor-agent -p` with the Cursor rule), `examples/codex/` (`codex exec`, a `config.toml` entry that reads the token from `MAIDAN_TOKEN` and gives a waiting claim room, and a prompt) and `examples/openrouter/claimant.py`, a standard-library runner that has any OpenRouter model do the coding while it runs the Maidan side itself: claim with `wait_ms`, renew, gate, deliver a `pi.change.result/1`, priced usage with `evidence: {provider, harness}`, review and release. The OpenRouter key is read from `OPENROUTER_API_KEY` only.

### The Claude Code claimant skill installs for your user, and names its harness

- **Changed:** `examples/claude-code/maidan-claimant/` works when copied to `~/.claude/skills/maidan-claimant/`: the runbook names no path in this repository, calls the helper as `$CLAIMANT_DIR/change_result.py`, and carries the MCP config inline. The README's install line copies it there.
- **Changed:** the runbook's `report_usage` always sends `evidence` with `provider` and `harness` (Claude Code: `{"provider": "anthropic", "harness": "claude-code"}`), which the swarm metrics slice on, and the fallback registers its own pull request with the MCP `register_thread_pull` instead of curl.
