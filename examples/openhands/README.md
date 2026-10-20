# OpenHands as a Maidan worker

[`maidan_claimant.py`](maidan_claimant.py) runs an agent on the
[OpenHands Software Agent SDK](https://docs.openhands.dev/sdk). Each run takes
one task off a Maidan channel, does it with OpenHands' terminal and file
editor, and hands the result back for a person to review. Maidan runs the
waiter loop around the work (claim, acknowledge, renew, result, review,
release); the task text is whatever a person or another agent filed on the
channel. It's the same loop as the [Goose recipe](../goose/).

## Run it

1. Give the agent its own identity: install it as an app with a token that
   holds `workspace:read`, `message:post` and `thread:transition`
   ([Integration, "An agent joins as an installed app"](../../docs/Integration.md)).
   Don't give it the admin token. Those three capabilities cover every tool
   the loop uses, approval gates included.
2. Install the SDK (Python 3.12 or later):

   ```sh
   python -m venv .venv && . .venv/bin/activate
   pip install -r examples/openhands/requirements.txt
   ```

3. Check the connection. This connects over MCP, lists what the token can
   see, and confirms the loop's ten tools are there. No model is called.

   ```sh
   export MAIDAN_URL=http://127.0.0.1:8080     # https for anything but this machine
   export MAIDAN_TOKEN=maid_...                # the agent's token; keep it out of files
   python examples/openhands/maidan_claimant.py --check
   ```

4. Work one task. The model name is in LiteLLM form:

   ```sh
   export LLM_MODEL=anthropic/claude-sonnet-4-5-20250929 LLM_API_KEY=...
   python examples/openhands/maidan_claimant.py --channel <channel uuid>
   ```

   `LLM_BASE_URL` points it at a gateway or a local model. A run with no work
   says "no work" and stops. To keep a worker on the channel, loop it:

   ```sh
   while true; do python examples/openhands/maidan_claimant.py --channel <channel uuid>; sleep 10; done
   ```

The agent works in the current directory, so run it where the task's files
are, such as a checkout of the repository the task is about.

## What the script asks of OpenHands

- **Only the tools the loop needs.** OpenHands sends every tool it holds on
  every model request ([Harness Caching](../../docs/Harness%20Caching.md#openhands)),
  and a worker token sees well over a hundred Maidan tools. The agent is
  filtered to the loop's ten, plus OpenHands' terminal and file editor.
- **Hand the thread to review, then release it.** Only `open` threads are
  handed out, so `transition_thread start_review` is what keeps finished work
  from coming back. The agent never closes a thread; a reviewer does.
- **Ask before acting irreversibly.** For a deploy, a deletion or a spend, the
  agent opens an approval gate on the thread and waits. It goes ahead only on
  `accepted`, and a person answers in `/ui`. Maidan refuses an approval from
  whoever requested it.
- **The token stays on this machine.** The script refuses to send it over
  plain http to anything but a loopback address.

The SDK negotiates MCP `2025-11-25` over streamable HTTP, which Maidan serves
at `/mcp/streamable`.
