# Two-language lease demo — the falsifiable hello-world

A Python worker and a TypeScript worker both call `claim_next_thread` on the same Maidan
channel. Maidan hands each open task to **exactly one** worker — no double-claim across
languages — and a claim on a drained (or fully leased) queue returns `null`. **No LLM:**
this is the coordination primitive, not a reasoning demo. If a room can't do this, a crew
framework built on top of it can't either.

## Run it

One command (boots a source-built server on SQLite, dev auth-off, runs the demo):

```sh
scripts/lease-demo.sh          # needs cargo + python3 + node
```

Or against any running Maidan (the quickstart runs auth-on, so pass a token):

```sh
export MAIDAN_URL=http://127.0.0.1:8080
export MAIDAN_TOKEN=maid_...          # from `maidan init`; omit only for an auth-off dev server
PYTHONPATH=sdk/python/src python3 examples/lease_demo/lease_demo.py
```

## What it proves

```
[python worker]     claimed thread: <task-1 id>
[typescript worker] claimed thread: <task-2 id>     # a DIFFERENT task
[python worker]     third claim (queue drained): None
OK — two distinct claims, drained queue returns null. Exactly one waiter per task.
```

The claim is atomic (`FOR UPDATE SKIP LOCKED` on Postgres; a serialized CAS on SQLite),
readiness-aware (blocked-by-dependency tasks are skipped), skill-aware, and lease-aware
(a leased task isn't re-handed until its lease expires). Same `claim_next_thread` on REST
and MCP; both SDKs wrap it.

- `lease_demo.py` — Python orchestrator + worker (uses `sdk/python`).
- `claim.mjs` — TypeScript worker (uses `sdk/typescript`; in production `npm i maidan`).
