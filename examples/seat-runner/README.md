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
the seat, the [runbook](../claude-code/maidan-claimant/runbook.md) in `$CLAIMANT_DIR` and the MCP connection.

The runner takes over three of the [runbook](../claude-code/maidan-claimant/runbook.md)'s sections, so the harness skips
them: "0b. Before every claim: the toolchain" (the runner's preflight),
"1. Claim" and "2. Acknowledge". The harness starts at "3. Read the task" and
ends with "6. Report, hand to review, release". The prompt names the
sections by title as well as number, so a renumbering is caught on reading.

`seats.example.toml` starts with cautious numbers for a small machine: one
seat, an 8 GB harness cap, a load ceiling at the CPU count. Tune them.

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
   sandbox, with the seat token and every `maidan.secret_env` variable
   removed from their environment, since they run code from the checkout.
   If one fails, nothing is claimed.
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
   runbook's "3. Read the task". The harness gets the seat token in its
   environment, because Codex's MCP config reads it from `token_env`; set
   `harness.pass_token = false` if your harness connects some other way.
6. **Release.** The claim is always released, whether the harness finished,
   crashed, hit a cap or was stopped, and only after every process the run
   started is dead. A claim the harness already released answers 409, which
   is fine. A release the server does not answer is retried
   `maidan.release_attempts` times with doubling backoff.

## `capped`

`capped` is `scripts/capped` in this repository. `capped --time S --mem-gb G
-- cmd` runs `cmd` in its own session and kills it at S seconds of wall clock
or G GB of memory. The runner reads its exit code: 124 means the time cap
fired (`timed_out`), 137 means the memory cap fired (`out_of_memory`), and
anything else is the command's own code. `capped` does not forward SIGTERM to
its command, so the runner signals the whole process tree itself.

Each run writes one JSON line to stdout. It has `seat`, `repo` and
`outcome`, plus `thread`, `exit` and `release` when a claim was taken. The
outcome is one of `ran`, `no_work`, `host_busy`, `preflight_failed`,
`timed_out`, `out_of_memory`, `failed`, `claim_failed`, `ack_failed`,
`queue_unreachable`, `stopped` or `error`. An unreachable server or an
unexpected error is logged and the seat carries on after `idle_secs`.

SIGTERM or SIGINT stops every seat. Each running harness and everything
below it gets SIGTERM, then SIGKILL after `kill_grace_secs`, and only then is
its claim released. `--once` does one run per seat and exits 0. A runner that
is itself SIGKILLed or crashes cannot release; `maidan.lease_secs` bounds how
long such a claim stays held.

Use an `https` URL for any server that is not on loopback: the runner warns
otherwise, because the bearer token would cross the network in clear.

## Prompts by kind

A seat takes whatever session work its repository has, changes and reviews
alike. `harness.prompt` is the prompt for a change; `harness.prompts` maps a
task kind (the claimed thread's `attributes.kind`) to its own prompt, e.g.
`prompts = { review = "examples/seat-runner/review-prompt.md" }` runs a review
thread through the [red-team runbook](../claude-code/maidan-claimant/red-team.md)
and posts the verdict on the pull request. A kind with no entry runs the
default prompt. A listed prompt file that does not exist is refused at startup.
Which work comes first is the queue's call: `claim-next` orders by priority and
age, so a producer marks a blocker's review with a higher `priority`.

## Models

`harness.models` lists the models this runner may start, and
`harness.model` must be one of them, e.g. `models = ["example-model"]`. A
model outside the list, or a config without the list, is refused at startup.

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
- a model that is not in `harness.models` is refused;
- SIGTERM kills the harness and releases its claim;
- an unanswered release is retried, and a seat survives a server that is down
  or an unexpected error;
- a stop between claim and launch, or a failed acknowledge, releases without
  starting the harness;
- a harness that ignores SIGTERM is SIGKILLed before its claim is released;
- the preflight never sees the seat token or `secret_env`.
