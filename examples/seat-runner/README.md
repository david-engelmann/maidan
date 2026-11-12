# Seat runner: keep session seats claiming

Session work sits unclaimed until someone starts a harness run. `seat_runner.py`
keeps N seats on the queue, one task per harness run, with the checks a
harness sandbox cannot do for itself. It needs only Python 3.11+.

```sh
export MAIDAN_TOKEN="$(cat ~/.config/maidan/seat-token)"
cp examples/seat-runner/seats.example.toml seats.toml   # fill it in
python3 examples/seat-runner/seat_runner.py --config seats.toml
```

A seat runs one of four harnesses, `[harness] name`: `codex` (Codex CLI),
`claude` (Claude Code), `gemini` (Gemini CLI) or `zcode` (Z.ai's ZCode CLI).
Set the harness up first: the seat token, the
[runbook](../claude-code/maidan-claimant/runbook.md) and its helper in
`$CLAIMANT_DIR`, and, for Codex, the MCP connection
([Codex](../codex/README.md)). The runner connects the other three to Maidan
itself ([Harnesses](#harnesses)).

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
   `engine: "session"` and the repo, then acknowledge. Each carries one
   `Idempotency-Key` across up to `maidan.claim_attempts` tries, retried on
   no answer (including one cut short or not JSON), 408, 429, 5xx and a key
   still in flight, waiting the server's `Retry-After` (at most a minute) or
   `maidan.claim_backoff_secs`, doubled per try. The key is what makes the
   retry safe: a claim whose answer was lost is answered again, not taken
   twice. A claim answer without a string `id` and `claim_lease_id` runs no
   harness; with nothing to release it by, its lease bounds it. A `null` claim ends
   the run. Only a thread whose footprint names the repo is claimed, so a
   producer gives each thread one (`PUT /threads/{id}/footprint`). A
   footprint needs at least one item: an empty `items` list clears it, and
   the thread is then never claimed. A review (`kind: review`) needs no
   footprint: its `repo` attribute is enough. The review child Maidan spawns
   carries its change's `repo` and `engine`, and a review a producer files
   can name `repo` the same way.
5. **Harness.** The whole harness process runs under
   `capped --time run_secs --mem-gb run_mem_gb`. A cap inside the sandbox
   could not measure anything. The harness gets the prompt
   ([`prompt.md`](prompt.md), or its kind's), plus the thread and lease, so
   it starts at the runbook's "3. Read the task". [Harnesses](#harnesses)
   has each command line. The harness gets the seat token in its
   environment, because each one's MCP config reads it from `token_env`;
   set `harness.pass_token = false` if your Codex connects some other way.
6. **Release.** The claim is always released, whether the harness finished,
   crashed, hit a cap or was stopped, and only after every process the run
   started is dead. A claim the harness already released answers 409, which
   is fine. A release the server does not answer is retried
   `maidan.release_attempts` times with doubling backoff.

## Harnesses

Codex, Claude Code and Gemini CLI run sandboxed, approve Maidan's tools up
front (nobody is there to answer a prompt) and make the repo's
`writable_roots` writable. ZCode has no sandbox ([below](#zcode)). The seat
token is on none of the command lines. Codex, Claude Code and Gemini read it
from `token_env` in their own environment. ZCode takes it only from a file,
written for the run and removed after it.

| `name` | Command | Maidan connection |
|---|---|---|
| `codex` | `codex exec -m <model> --sandbox workspace-write --ephemeral -c mcp_servers.maidan.default_tools_approval_mode="approve" -c sandbox_workspace_write.writable_roots=[…] <prompt>` | `~/.codex/config.toml` ([`config.toml`](../codex/config.toml): `bearer_token_env_var`) |
| `claude` | `claude -p <prompt> --model <model> --permission-mode acceptEdits --allowedTools 'mcp__maidan__*' --strict-mcp-config --mcp-config <json> --settings <json> --no-session-persistence --add-dir <root>…` | Inline in `--mcp-config`, so no file holds it: an `http` server whose `headersHelper` prints `Authorization: Bearer` from `token_env` |
| `gemini` | `gemini -p <prompt> -m <model> --approval-mode yolo --sandbox --allowed-mcp-server-names maidan --include-directories <roots>` | A settings file the runner writes for the run (below) |
| `zcode` | `zcode --prompt <prompt> --cwd <checkout> --mode guarded --disallowed-tools "AskUserQuestion EnterPlanMode ExitPlanMode" --no-color` | A runtime config in a private `HOME` the runner writes for the run ([below](#zcode)) |

- **Claude Code.** `--strict-mcp-config` drops every other MCP server.
  `--settings` turns on Claude Code's Bash sandbox, with no retry outside it
  and a refusal to start where it is unavailable (`failIfUnavailable`).
  Sandboxed commands write to the checkout, the temp directory and each
  `--add-dir`, and reach only `harness.allowed_domains` (default: the Maidan
  host; add `github.com` or a package registry when the preflight's
  toolchain needs them). `acceptEdits` takes file edits without asking.
- **Gemini CLI.** Gemini reads MCP servers only from a settings file. The
  runner writes one per run, `settings.json` with mode 0600 in a fresh 0700
  directory under `harness.state_dir` (default
  `$TMPDIR/maidan-seat-runner-<uid>`), and points
  `GEMINI_CLI_SYSTEM_SETTINGS_PATH` at it. It holds Maidan's URL and
  `"Authorization": "Bearer ${MAIDAN_TOKEN}"`, the variable's name and not
  the token, which Gemini expands from its environment. The directory is
  removed once the harness and everything it started are dead, whether it
  finished, crashed, hit a cap or was stopped; a runner that is itself
  SIGKILLed leaves it, and the next runner to start removes it.
  `GEMINI_CLI_TRUST_WORKSPACE=true` skips the folder-trust check nobody
  could answer. On macOS `--sandbox` is Seatbelt, which makes the checkout
  and at most five `--include-directories` writable, so a gemini repo takes
  at most five `writable_roots`.
- **Endpoint.** `harness.mcp_path` picks the endpoint for `claude`,
  `gemini` and `zcode`: `/mcp/streamable` (the default), `/mcp/worker` or
  `/mcp/reviewer`. The runbooks call tools outside the worker and reviewer
  profiles (`list_messages`, `get_thread_result`,
  `list_result_deliveries`, `register_thread_pull`; a review renews and
  releases its claim and reports usage), so narrow it only for a prompt that
  stays inside one. Codex takes its endpoint from its own config.

### ZCode

[ZCode](https://github.com/zai-org/ZCode) is Z.ai's own coding agent and
the GLM coding plan's native harness. Its CLI has no flag for the model or
for a config file (`--model` and `--settings` do not exist; its CLI
reference says not to invent them), so the runner writes two files per run,
each 0600, under a fresh 0700 directory in `harness.state_dir` and never in
the checkout:

- **The provider file,** `provider_config.json`, which
  `ZCODE_PERSONAL_PROVIDER_CONFIG_FILE` points at: one personal provider
  with `api.type` (`harness.api_type`, default `anthropic-messages`),
  `api.baseUrl` (`harness.api_base_url`, default z.ai's
  `https://api.z.ai/api/anthropic`), the model as `defaultModelSelection`,
  and the key from the variable `harness.api_key_env` names. ZCode does not
  expand variables there, so the key is in the file for the run. The
  variable itself is kept from the preflight and from the harness's
  environment.
- **The runtime config,** `home/.zcode/cli/config.json`: `features.mcp` on
  and one `http` server, `maidan`, with `Authorization: Bearer <token>` in
  `headers`. ZCode reads MCP servers only from that path under `HOME` (or
  from the project's `zcode.json`, which is in the checkout), and does not
  expand variables in headers, so the run gets `HOME` and
  `ZCODE_DATA_BASE_DIR` set to that private `home` and the token is in this
  file for the run. A private `HOME` would also move the toolchain's state,
  so `CARGO_HOME`, `RUSTUP_HOME`, `GIT_CONFIG_GLOBAL` and `XDG_CONFIG_HOME`
  stay on the real home unless already set; set others a repo needs in
  `harness.env`. `$CLAIMANT_DIR` is written into the prompt as an absolute
  path for the same reason.

Both files go when the run's directory goes, as for Gemini: after the run,
a crash, a cap or a stop, and, after a SIGKILLed runner, when the next
runner starts.

**Guarded, not yolo.** ZCode has no OS sandbox, and the runner does not add
one, so a zcode seat runs with this user's full rights; it takes no
`writable_roots`, which would promise a limit it does not have. `--mode
guarded` runs ordinary tools without asking and asks before the commands its
rules call dangerous (a force push, `git clean -f`, `git checkout` over the
tree, `rm -rf`, `find -delete`, `rsync --delete` and the like). With nobody to
ask, those calls fail and the run goes on, which is what the runbook wants
anyway. `--mode yolo`, which would run them, is never used. An old ZCode
build without `guarded` refuses the flag, and the run fails rather than
running unguarded.

**Install** from source, as its
[quickstart](https://github.com/zai-org/ZCode/blob/main/apps/zcode-cli/docs/usage/01-quickstart.md)
says (Node and pnpm at the versions in its `mise.toml`):
`pnpm install --frozen-lockfile && pnpm --filter @zcode/cli... build`, then
`bin = ["node", "/abs/ZCode/apps/zcode-cli/packages/cli/dist/zcode.cjs"]`,
or the single-file build (`pnpm --filter @zcode/cli sea --target
darwin-arm64`) as `bin`. Z.ai publishes no package: the `zcode-cli` on npm
is not theirs.

## Providers and families

A seat says who serves its model, `harness.provider`, and Maidan reads a
model family from that and the model: `anthropic`, `openai`, `google`,
`xai`, `deepseek` or `zhipu` (GLM). Second-family review depends on it: a
review seat declines a change of its own family, and Maidan refuses a
review result from the author's family. The prompts fill `{provider}`,
`{harness}`, `{family}`, `{model}` and `{claimant_dir}` with the seat's own
values, so each run reports `evidence: {"provider": …, "harness": …}` and
makes its own-family decline with its own family.

| `name` | `provider` default | `harness` in the evidence | `claimant_dir` default |
|---|---|---|---|
| `codex` | `openai` | `codex` | `~/.codex/maidan-claimant` |
| `claude` | `anthropic` | `claude-code` | `~/.claude/skills/maidan-claimant` |
| `gemini` | `google` | `gemini-cli` | `~/.gemini/maidan-claimant` |
| `zcode` | `z.ai` | `zcode` | `~/.zcode/maidan-claimant` |

`harness.family` defaults to the family Maidan reads; a seat that sets
another, or whose provider and model name no family, is refused at startup.

**GLM's coding plan** runs in ZCode, its native harness, or in Claude Code
pointed at z.ai's Anthropic-compatible endpoint. Either way the key comes
from a variable you name, never from the config file. In ZCode:

```toml
[harness]
name = "zcode"
bin = ["node", "/abs/ZCode/apps/zcode-cli/packages/cli/dist/zcode.cjs"]
models = ["glm-5.3"]
model = "glm-5.3"                 # provider defaults to z.ai, so the family is zhipu
api_key_env = "ZAI_API_KEY"
```

In Claude Code, the key goes in `env_from` (a literal `env` value whose name
looks like a key is refused):

```toml
[harness]
name = "claude"
provider = "z.ai"                 # Maidan reads z.ai and glm-… as zhipu
models = ["glm-5.3"]
model = "glm-5.3"
env = { ANTHROPIC_BASE_URL = "https://api.z.ai/api/anthropic", API_TIMEOUT_MS = "3000000" }
env_from = { ANTHROPIC_AUTH_TOKEN = "ZAI_API_KEY" }   # Claude Code's name = the variable holding the key
```

A `claude` seat whose `ANTHROPIC_BASE_URL` (in `env` or inherited) is not
Anthropic's but whose provider is still `anthropic` is refused, and so is a
non-Anthropic provider without such an endpoint: either way Maidan would
read the wrong family. Such a seat does not inherit `ANTHROPIC_API_KEY` or
`ANTHROPIC_AUTH_TOKEN` from the runner's environment, so an Anthropic key
never goes to the other endpoint. Each `env_from` source is kept from the
preflight, like the seat token. Name a GLM model (`glm-…`), not a Claude
alias that z.ai maps: Maidan reads `claude-…` as anthropic first.

**A Google AI plan** runs in Gemini CLI, signed in as the plan's account
(or with `GEMINI_API_KEY` in `env_from`). Google moved unpaid and Google One
accounts from Gemini CLI to Antigravity CLI on 2026-06-18; a paid plan or an
API key still runs Gemini CLI.

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
`queue_unreachable`, `queue_unreadable`, `stopped` or `error`, with
`detail` saying why when it is a failure. `queue_unreadable` is a
queue-depth answer without a whole-number `ready`: it is not read as an empty
queue. An unreachable server or an unexpected error is logged and the seat
carries on after `idle_secs`. Every request gives up after
`maidan.timeout_secs` (default 60) without an answer.

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
- the preflight never sees the seat token or `secret_env`;
- each harness (`codex`, `claude`, `gemini`, `zcode`) builds its command,
  and `codex`'s is unchanged;
- the seat token and a model key are on no command line and in no file a
  run leaves; a gemini or zcode run's files are 0600 in 0700 directories
  outside the checkout and are gone after the run, after a crash, after a
  stop, and (swept by the next runner) after the runner is SIGKILLed;
- a zcode seat runs `--mode guarded`, gets its key only in its provider
  file, and is refused with a mismatched endpoint, no key, or
  `writable_roots`;
- a `claude` seat pointed away from Anthropic but still claiming to be
  Anthropic is refused, and a GLM seat gets its own key and no Anthropic key;
- each prompt renders with its seat's own provider, harness and family;
- the family table matches Maidan's `PROVIDER_FAMILIES`.

`scripts/test-mcp-fault-injection.py` puts the runner behind
`scripts/mcp-fault-proxy.py` and injects one fault per test: latency, a
timeout, a dropped connection, a 500/502/503 with and without `Retry-After`,
an answer cut short, a JSON-RPC error and a drifted schema. Each run either
recovers within its tries or stops with one line, holding no claim it could
release and never claiming twice.
