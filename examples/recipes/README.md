# Compose recipes

Two agents you can start with one command. Each one boots Maidan built from this
checkout, files a task, and runs an agent that works through it. Both use the
waiter loop from [Integration](../../docs/Integration.md#the-waiter-loop): a leased
claim that is kept alive while the agent works and released when it finishes, a
result on the thread, and nothing left assigned to an agent that has gone away.

| Recipe | What it shows | Speaks |
|--------|---------------|--------|
| [`coding-agent.yaml`](coding-agent.yaml) | Claim a task, run a coding agent on it, hand back its patch as an artifact | REST |
| [`deploy.yaml`](deploy.yaml) | Claim a deploy, open an approval gate, and deploy only after a person accepts | REST + MCP |

For MCP clients (Cursor, Claude) and A2A, see the configs and `a2a_interop.py` in
[`examples/`](../README.md).

## Run one

Docker Compose 2.20 or later. The first run compiles Maidan (several minutes);
later runs reuse the image.

```sh
docker compose -f examples/recipes/coding-agent.yaml up --build
```

```
provision-1     | provision: coder (…) may workspace:read, message:post, thread:transition, artifact:upload; filed 'Add a CONTRIBUTORS file' in #coding
coding-agent-1  | coding-agent: claimed …: Add a CONTRIBUTORS file listing the maintainers, one per line.
coding-agent-1  | coding-agent: done …, artifacts ['…']
```

```sh
docker compose -f examples/recipes/deploy.yaml up --build
# the agent now waits on a gate; in another terminal, be the person who says yes:
docker compose -f examples/recipes/deploy.yaml run --rm approve            # or --decline
```

```
deploy-agent-1  | deploy-agent: … waiting on gate …
deploy-agent-1  | deploy-agent: deployed
```

Both bind to `127.0.0.1:8080`; set `MAIDAN_HOST_PORT` if it is taken. File your
own task with `RECIPE_TASK="…"` on the first `up`, or post more tasks to the
channel as the admin (`docker compose -f … exec maidan cat /data/recipe/init.txt`
shows its token, which also opens `/ui`). Start over with `down -v`.

When an agent finishes, it hands the thread to review and lets go of it. Only
`open` threads are handed out, so finished work stays done; closing it is a
reviewer's call.

## Plug in a real agent

Without a command, each recipe runs a stand-in: the coding agent writes a patch
that records the task, and the deploy agent prints what it would deploy. Neither
is an LLM, and the output says so.

- **`AGENT_COMMAND`** runs in an empty working directory with the task in
  `$MAIDAN_TASK`. Every file it leaves in `$MAIDAN_OUTPUT_DIR` is uploaded and
  attached to the thread. Exit 0 is `done`, anything else `failed`.
- **`DEPLOY_COMMAND`** runs only after the gate is accepted, with the task in
  `$MAIDAN_TASK`.

The agent images are stock `python:3.13-slim`, so a real agent CLI (and its
credentials) needs an image that has it. Replace the service's `image:`.

## What each piece does

| File | Role |
|------|------|
| `base.yaml` | Maidan (auth on, SQLite, loopback only) and a one-shot `maidan init` |
| `Dockerfile.source` | The quickstart image's layout, built from this checkout |
| `provision.py` | As the admin: create the agent's member, mint it a token scoped to its job, file the task |
| `coding_agent.py`, `deploy_agent.py` | The agents |
| `approve.py` | The terminal version of the `/ui` approval card |
| `maidan_http.py` | A standard-library client (REST, stateless MCP, the claim lifecycle) |

The admin token never reaches an agent's container. It stays on the server's
data volume, where `provision` and `approve` read it. Each agent gets only its
own token, through a volume that nothing else mounts. The deploy agent cannot
answer its own gate: its installation does not grant `workspace:write`, and a
token that did would still be refused, because Maidan never accepts an approval
from whoever requested it.

These are recipes for trying Maidan, not deployments. For production, use
`compose.yaml` (Postgres and an object store) or the Helm chart, as described in
[Production](../../docs/Production.md).
