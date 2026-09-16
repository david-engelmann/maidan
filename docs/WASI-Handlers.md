# WASI slash handlers

A workspace can install a sandboxed WebAssembly module as a slash-command
handler. Someone types `/report last-week` in a thread, Maidan runs your
module, and what the module writes to stdout becomes the command's response.

**The guest is the tool, not an agent.** It gets one invocation, no network, no
filesystem, a fuel budget and a memory cap, and then it exits. If you want
something that holds a conversation, claims threads and calls back in, that is
an agent over [MCP](Integration.md#transports) — not this.

Use a WASI handler when the work is a pure function of its inputs: format a
table, validate a spec, compute a diff, render a checklist. Use an `http`
handler when it needs to reach anything.

## Install one

Three steps, in this order. The order matters: a registration names bytes by
content hash, so the bytes have to exist first.

### 1. Build a module

Any language that targets `wasm32-wasip1` works. The module needs a `_start`
export, which every WASI toolchain emits for a `main`.

```sh
# Rust
rustup target add wasm32-wasip1
cargo build --release --target wasm32-wasip1
# -> target/wasm32-wasip1/release/my_handler.wasm
```

```sh
# TinyGo
tinygo build -o my_handler.wasm -target=wasip1 ./main.go
```

### 2. Upload it as an artifact

```sh
curl -X POST "$MAIDAN/artifacts?kind=attachment" \
  -H "Authorization: Bearer $TOKEN" \
  --data-binary @my_handler.wasm
# -> {"sha256":"9f2b…","size":184320,…}
```

This needs `artifact:upload`. The upload records a per-workspace access link
(Cluster 204) — that link, not the bytes, is what makes the module yours.
Artifacts are content-addressed and deduplicated across the whole instance, so
two workspaces uploading identical bytes each get their own link to one blob.

### 3. Register the command

```sh
curl -X POST "$MAIDAN/workspaces/$WS/slash-commands" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"report","handler_kind":"wasi","handler_target":"9f2b…"}'
```

`handler_target` is the sha256 from step 2, raw hex or `sha256:`-prefixed.
This needs `workspace:write`. The MCP twin is `register_slash_command` with the
same fields.

Registration **verifies your workspace owns that sha** and returns `400` if it
does not. That is deliberate: a command you can register but that can never run
is worse than a rejected form, because the failure surfaces later, to a
different person, as a broken command. The same check runs again at dispatch,
because a workspace can lose an artifact after a registration was accepted.

## The ABI

`$type` is the contract; a breaking change is a new `/2` type, never a
redefinition of this one.

### In: stdin

One JSON object, `maidan.slash.wasi-invoke/1`:

```json
{
  "$type": "maidan.slash.wasi-invoke/1",
  "command": "report",
  "args": "last-week",
  "workspace_id": "…", "channel_id": "…", "thread_id": "…",
  "author_id": "…", "message_id": "…"
}
```

`args` is the raw remainder of the line after the command name — parsing it is
yours. The ids are context, not authority: a guest cannot call back into Maidan,
so they are there to be echoed into your output or used as cache keys.

`argv` and the environment are **empty on purpose**. Mirroring the same ids into
a second channel would be two copies of one fact with no reader.

### Out: stdout, and the exit status

Whatever you write to stdout is the response text. Exit `0` (or return from
`main`) for success. stderr is captured and reported alongside, so it is a fine
place for diagnostics.

Exit non-zero to fail on your own terms — the status reaches the room as
`exit_code`, distinct from a crash.

## What a guest can do

Exactly 16 `wasi_snapshot_preview1` calls resolve:

| Group | Calls |
|---|---|
| stdio | `fd_read` `fd_write` `fd_close` `fd_seek` `fd_fdstat_get` |
| filesystem stubs | `fd_prestat_get` `fd_prestat_dir_name` (both report *no preopens*) |
| args / env | `args_get` `args_sizes_get` `environ_get` `environ_sizes_get` (all empty) |
| clocks | `clock_time_get` `clock_res_get` |
| other | `random_get` `proc_exit` `sched_yield` |

Everything else — `path_open`, `sock_connect`, `fd_readdir`, any import from any
other module — is refused **before instantiation**, so a banned import is a
registration-time class of mistake rather than a runtime one.

This is not a filter over a larger surface. The host implements these calls and
only these, so there is nothing to bypass: a module that imports `path_open`
fails to link because no such function exists to link against. The two
filesystem stubs answer "no preopens", which is what makes the guest see an
*empty* filesystem rather than a restricted one.

`random_get` returns zeros. A handler that needs entropy should take it as input;
a pure function of its inputs is the thing this surface is for.

## What bounds a run

| Bound | Default | Ceiling |
|---|---|---|
| Fuel (interpreter instructions) | 25,000,000 | 100,000,000 |
| Linear memory | 16 MiB | 64 MiB |
| Captured output (host) | 256 KiB | — |
| Output carried into the room | 16 KiB | — |

Fuel is the real wall. No allowlisted import blocks — stdin is a buffer,
`sched_yield` returns immediately, the clocks do not sleep — so a guest cannot
wait, and a runaway ends as fuel exhaustion rather than a hang.

The two output bounds answer different questions. The first protects host memory
while a run is in flight. The second protects the event log: a slash response is
written into the triggering message's metadata and fanned out to every
subscriber, so it is persisted and replicated, not just held. Either cut is
marked in the text — a clipped response never reads as a complete one.

## When it fails

Every failure names a cause, because "fuel_exhausted" and "trap" are the
difference between shrinking a loop and fixing a crash.

| `error_kind` | What happened | What to do |
|---|---|---|
| `fuel_exhausted` | Ran out of instruction budget | Do less work, or move it to an `http` handler |
| `memory_limit` | Grew past the linear-memory cap | Stream instead of buffering |
| `exit_non_zero` | Your module called `proc_exit(n)` | Your own error path — `exit_code` carries `n` |
| `trap` | Crashed: unreachable, bad indirect call, out-of-bounds | A bug in the guest |
| `banned_import` | Imports something off the allowlist | Drop the dependency reaching for it |
| `invalid_module` | Not valid wasm, no `_start`, or the sha is not yours | Rebuild, or upload before registering |

A failure still carries whatever the guest managed to write before it failed.
That partial output is usually the whole diagnosis, so print your progress.

Causes are reported by precedence, not by whichever surfaced last: a
host-enforced limit outranks the guest's own verdict. A module that hits the
memory cap, fails its next allocation and then exits `1` is reported as
`memory_limit` — the cause, not the symptom.

## What the room sees

The response lands in the triggering message's `metadata.slash_response`, the
same shape every handler kind produces, and the `/ui` renders it inline under
the message. A failure shows the kind first, then the message, then the exit
code if there was one.

## Not supported

FSM hooks refuse `wasi` on both write surfaces. A hook fires on a state
transition with no one waiting on the answer, so there is nowhere to report a
guest's output or its failure — an `http` hook is the shape that works there.
