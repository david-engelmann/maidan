# Any OpenRouter model as a Maidan claimant

[`claimant.py`](claimant.py) is a one-shot claimant that runs any model on
[OpenRouter](https://openrouter.ai) (Grok, GPT, Claude, Gemini, open-weight
models) against one Maidan task. Unlike the Claude Code, Cursor and Codex
recipes, the model never sees Maidan. The script follows the shared
[claimant runbook](../claude-code/maidan-claimant/runbook.md) in code, and
the model only does the coding, with five tools: read, write and list files
in the checkout, run a command, and post a progress note. Standard library
only.

What one run does:

1. Claims with `claim_next_workspace_thread` (`engine: "session"`, this
   checkout's `repo`, `wait_ms` up to 300000). No work by the deadline: it
   says "no work" and exits 0.
2. Acknowledges, then renews the lease every third of it while the model
   works. If a renewal is refused, it delivers nothing.
3. Reads the context, the opening message's `deliver_to`, the attributes
   (`GET /threads/{id}`) and any previous result. It checks out the agent
   branch: the existing one for a follow-up, else a new one from the base.
   It declares the branch in the footprint.
4. Lets the model work until it calls `finish`, then runs the gate
   (`--gate`, default `make prepush`). A failing gate goes back to the
   model once (`--gate-retries`).
5. Commits, builds the `pi.change.result/1` with the runbook's
   `change_result.py`, and `PUT`s it, so Maidan opens the draft pull request.
   For a change the flow cannot commit (the helper's exit 3), `--allow-push`
   takes the runbook's fallback: its own branch, a draft via `gh`, and
   `register_thread_pull`. Without that flag the result is a `seat_error`
   saying why.
6. Reports usage, priced from OpenRouter's model catalog, with
   `evidence: {"provider": "<vendor>", "harness": "openrouter"}`. The vendor
   comes from the model id (`x-ai/grok-4` gives `xai`), or from
   `--provider`. Then it starts review and releases the claim, which it
   always does, even after a failure.

## Run it

```sh
export MAIDAN_URL=http://127.0.0.1:8080     # https for anything but this machine
export OPENROUTER_API_KEY=...               # from your shell or a secret store; never in a file here
# the seat token: MAIDAN_TOKEN, else ~/.config/maidan/seat-token (runbook step 0)

cd /path/to/the/checkout
python3 /path/to/maidan/examples/openrouter/claimant.py --model x-ai/grok-4 --check
python3 /path/to/maidan/examples/openrouter/claimant.py --model x-ai/grok-4 --base dev
```

`--check` connects, confirms the seat can see the 14 Maidan tools the script
uses, and confirms the key is set. It calls no model and claims nothing. The
script refuses to send the token over plain http to anything but this
machine. It reads the OpenRouter key from `OPENROUTER_API_KEY` only.

The model runs commands in the checkout. Run the script where that is
acceptable: a container or a disposable clone, not a machine holding
credentials the task has no business with. The script refuses model commands
that push, commit, rebase or reset, and file edits outside the checkout or
under `.git`. That is a guard against mistakes, not a sandbox.

To keep a seat on the queue, run it again when it exits. Each run waits for
work in the claim itself, so there is no sleep between runs.
