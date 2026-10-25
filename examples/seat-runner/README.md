# Seat runner: keep session seats claiming

Session work sits unclaimed until someone starts a harness run. `seat_runner.py`
keeps N seats on the queue, one task per harness run, with the checks a
harness sandbox cannot do for itself. It needs only Python 3.11+.

```sh
export MAIDAN_TOKEN="$(cat ~/.config/maidan/seat-token)"
cp examples/seat-runner/seats.example.toml seats.toml   # fill it in
python3 examples/seat-runner/seat_runner.py --config seats.toml
```

Set up the harness first, as in its recipe ([Codex](../codex/README.md)):
the seat, the runbook in `$CLAIMANT_DIR` and the MCP connection.

## One run

Each seat takes the configured repositories in turn. For each run:

1. **Host check, outside the sandbox.** The 15-minute load is under
   `host.max_load15` and the checkout's disk has `host.min_free_gb` free.
   Codex's `workspace-write` sandbox denies `ps`, so it can't check either.
2. **Queue poll.** `GET /workspaces/{wid}/queue-depth?engine=session&repo=…`.
   If nothing is ready for this seat, there is no run, because a harness run
   costs money.
3. **Preflight.** The repo's `preflight` commands (the toolchain's version,
   then the repository's own narrow check) run under `capped`, outside the
   sandbox. If one fails, nothing is claimed.
4. **Claim.** `POST /workspaces/{wid}/threads/claim-next` with
   `engine: "session"` and the repo, then acknowledge. A `null` claim ends
   the run.
5. **Harness.** The whole harness process runs under
   `capped --time run_secs --mem-gb run_mem_gb`. A cap inside the sandbox
   could not measure anything. Codex runs as
   `codex exec -m <model> --sandbox workspace-write --ephemeral
   -c mcp_servers.maidan.default_tools_approval_mode="approve"
   -c sandbox_workspace_write.writable_roots=[…]` with
   [`prompt.md`](prompt.md), plus the thread and lease, so it starts at the
   runbook's step 3.
6. **Release.** The claim is always released, whether the harness finished,
   crashed, hit a cap or was stopped. A claim the harness already released
   answers 409, which is fine.

Each run writes one JSON line to stdout. It has `seat`, `repo` and
`outcome`, plus `thread`, `exit` and `release` when a claim was taken. The
outcome is one of `ran`, `no_work`, `host_busy`, `preflight_failed`,
`timed_out`, `out_of_memory`, `failed`, `claim_failed`, `queue_unreachable`
or `stopped`.

SIGTERM or SIGINT stops every seat. It kills each running harness with
everything below it, because `capped` does not forward a signal to its
command, and releases the claims. `--once` does one run per seat and exits 0.

## Models

`MODEL_ALLOWLIST` in `seat_runner.py` names the models each harness may run.
For Codex that is `gpt-5.6-sol`. Any other model, `gpt-6-astra` included, is
refused at startup.

## Tests

```sh
python3 -m unittest discover -s examples/seat-runner/tests
```

The tests run against a fake Maidan, a fake `capped` and a fake harness. They
check that:

- a failed preflight claims nothing;
- a timeout releases the claim;
- a `null` claim exits 0;
- an empty queue or a busy host claims nothing;
- a model that is not on the allowlist is refused;
- SIGTERM kills the harness and releases its claim.
