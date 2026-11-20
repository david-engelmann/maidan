# Advice from a decision model

When a task is handed to review, Maidan can ask a decision model what it reads
in the hand-off, and show the reviewer the answers. The model is
[TypeSafe](https://typesafe.ai)'s Jev, a System One model: it answers typed
questions with calibrated probabilities instead of writing text.

The design has one rule: **the model makes judgments and code makes the
decision.** Maidan never asks the model whether the work should land. It asks
ten yes/no questions, each about one thing a person could point to, and a
table in the server turns the ten probabilities into green, amber or red. The
questions and the table are both in
`crates/maidan-server/src/land_gate_advisor.rs`, and nowhere else.

The advice is advice. It does not approve, close, or write the land gate. A
reviewer, a person or another agent, still decides, and an approval gate still
waits for whoever it names.

It is off unless the instance turns it on, and it needs a TypeSafe API key:
the instance's, or the workspace's own.

## What the model reads

The server builds the packet from the thread, so every call sends the same
shape and a caller cannot choose what leaves:

```json
{
  "task": {
    "title": "Rate-limit the public API",
    "description": "60 requests a minute per token, then 429."
  },
  "result": {
    "status": "changed",
    "summary": "Adds a token-bucket limiter; 14 new tests pass."
  },
  "agent_messages": ["The limiter is in and the tests pass."]
}
```

- `task` is the thread's title and description, and its `acceptance` when
  the task's attributes say how a reviewer checks it. The questions about
  what the task asks for are read against all three.
- `result` is the result the agent recorded, without `diff`, `deliver_to` and
  `base_sha` (the bytes the change flow needs, not a description of the work),
  and with any string over 6,000 characters cut.
- `agent_messages` are the last ten messages on the thread from the member
  that produced the result, each cut at 2,000 characters. Nobody else's
  messages are sent, and the field is left out when there are none.

The task's opening message is not sent, and that was measured. An opening
message tells the agent how to work (claim the task, say what you ran, send
it to review), and a hand-off cannot show those steps. With it in the packet,
"does what the task asks" went unsettled and the advice left its label on
four of sixteen labelled hand-offs; without it, on none. So say what a task
asks for in its description and how to check it in `acceptance`: MCP
`create_thread` takes both.

That is everything that leaves the deployment, and it goes only to the host in
`MAIDAN_JEV_BASE_URL`.

## The ten judgments

Each is one question with a yes and a no described, sent together in one
request. The model answers each with the probability of yes. At 0.7 or above a
judgment reads as yes, at 0.3 or below as no, and in between it is unsettled.

| Judgment | The question |
|---|---|
| `done` | Do the result and the agent's messages show that the agent did what the task asks for? |
| `tests_passed` | Do they say that tests or checks ran and passed? |
| `code_change` | Does the task ask for a change to code? |
| `unresolved_failure` | Do they report a failing test, an error or a broken check that is still not fixed? |
| `left_undone` | Do they say that part of what the task asks for was not done? |
| `asks_person` | Do they ask a person a question that needs an answer before the work is finished? |
| `beyond_task` | Does the result report a separate change that the task did not ask for? |
| `changes_access` | Did the agent's own work change the code or settings that control sign-in or permissions? |
| `deletes_data` | Does the result report that the agent deleted stored data, or added something that deletes it? |
| `flags_risk` | Does the agent warn about a risk or a consequence of this work that someone should deal with? |

## The decision, in code

| Advice | When |
|---|---|
| **Red** | `unresolved_failure` is yes; or `done` is no and `beyond_task` is yes (the hand-off is about other work) |
| **Green** | `done` is yes; `unresolved_failure`, `left_undone`, `asks_person`, `beyond_task`, `changes_access`, `deletes_data` and `flags_risk` are all no; and either `code_change` is no or `tests_passed` is yes |
| **Amber** | everything else |

An unsettled judgment never counts toward green: a doubt about the work or
about any warning sign is amber, a task that may ask for a code change needs
its tests, and tests that may have passed are read as not reported. A change to sign-in or permissions, a
deletion of stored data and a warning from the agent are amber by policy, not
because the model disapproves: those are the hand-offs a person should look
at. The answer lists everything that kept it from green (`reasons`), with
what made it red first, and marks the judgments behind each reason.

## Asking

```sh
curl -X POST "$MAIDAN/threads/$THREAD/land-gate/advice" \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' -d '{}'
```

MCP: `advise_land_gate` with `thread_id`. Both need `thread:transition` and a
thread the caller can read.

```json
{
  "provider": "typesafe",
  "model": "jev-1.14.0",
  "land": "amber",
  "reasons": ["tests_not_reported"],
  "judgments": [
    { "id": "done", "label": "Does what the task asks", "probability": 0.94, "answer": "yes", "reading": "Does what the task asks", "concern": false },
    { "id": "tests_passed", "label": "Reports passing tests", "probability": 0.01, "answer": "no", "reading": "Reports no passing tests", "concern": true }
  ],
  "thresholds": { "yes_min": 0.7, "no_max": 0.3 },
  "key_source": "workspace",
  "cached": false,
  "usage": { "input_tokens": 512, "output_tokens": 10 },
  "latency_ms": 148
}
```

Each judgment carries its `reading`: the answer as a statement ("Reports no
passing tests"), so a reader does not have to work out whether a yes is good.
One reading depends on the rules: when a task asks for no code change, missing
tests are no concern, and the row says so ("Reports no tests, which this task
does not call for").
The model is as sure of that statement as the probability of the answer it
reads as: 1% yes is a no it is 99% sure of.

An answer is held for fifteen minutes under the exact packet it was given
for, so a sheet opened twice asks once; a changed result or a new message
from the agent asks again, and `{"refresh": true}` always does.

| Status | Meaning |
|---|---|
| 404 | The instance does not offer advice |
| 409 | The task has no result yet, or the workspace has no key to ask with |
| 502 | The provider failed or timed out. Only the advice call fails; the gate is unchanged |

In the console, a task under **Needs review** shows the advice on its sheet:
"Nothing flagged" for green, "Worth a look" for amber and "Problem reported"
for red, the reasons in words, and the ten judgments one click away. The
words say what the model read, not what to do, and the panel says it approves
and blocks nothing.

## Keys

A call uses the workspace's own key when it stored one, and the instance's
key otherwise. With neither, advice is refused.

```sh
# Store this workspace's key (secret:admin). The provider is asked to accept
# it first; a key it refuses is not stored.
curl -X PUT "$MAIDAN/workspaces/$WS/land-gate/advisor" \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"api_key": "…"}'
# What is set up: {"enabled": true, "key_source": "workspace", "model": "jev-latest", "host": "api.typesafe.ai"}
curl "$MAIDAN/workspaces/$WS/land-gate/advisor" -H "authorization: Bearer $TOKEN"
# Forget it.
curl -X DELETE "$MAIDAN/workspaces/$WS/land-gate/advisor" -H "authorization: Bearer $TOKEN"
```

The key is kept as the workspace secret `typesafe_api_key`, encrypted at rest
(the instance needs `FEDERATION_ENCRYPTION_KEY`), and no route returns it. In
the console it is under **Tools**, **Workspace**.

On maidan.world a workspace brings its own key. try.maidan.world asks with the
demo's: when the page's scripted agent has sent **Say hello** to review, the
page asks for advice on that hand-off with the sandbox's own token
(`advise_land_gate`) and shows the answer, the ten judgments included, before
it sends the visitor to the console to approve. The task's sheet there shows
the same answer, held from that call. When the demo has no key, or the model
does not answer within four seconds, the page shows nothing and goes on. Where the
demo has a key, the page says before a sandbox exists that tasks sent to
review there are read by Jev (`advice` in `/try-config.json`).

## Turning it on

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_JEV_LAND_GATE_ENABLED` | `0` | `1` turns the routes and the tool on. An invalid value refuses boot. With it off, none of the others are read |
| `TYPESAFE_API_KEY` | none | The instance's own key, for workspaces that stored none. Leave it unset to have every workspace bring its own |
| `MAIDAN_JEV_MODEL` | `jev-latest` | A model or alias from TypeSafe's `GET /v1/models` |
| `MAIDAN_JEV_TIMEOUT_MS` | `1500` | Per request. A timeout fails only the advice call, with `502` |
| `MAIDAN_JEV_BASE_URL` | `https://api.typesafe.ai/` | An origin only, behind the same egress guard as other outbound calls. The name is resolved on the first call and again every ten minutes: a provider that cannot be reached fails the advice call, never the server |

## Measuring it

`scripts/eval-land-gate-advisor.py` runs labelled hand-offs through a running
Maidan and reports, for every judgment and for the advice, how many answers
agree with the labels. The labelled set it ships with,
`scripts/land-gate-advice-cases.json`, is 36 synthetic hand-offs across code
changes, documentation, investigation, support replies and infrastructure
work, written to include the hard cases: tests that were not run, a question
left in a message, work on the wrong thing, a result that tells the reader
how to answer.

Use your own hand-offs before you rely on it: a model that reads our examples
well has not been shown to read yours.

## What it is not

- It does not read the diff. It reads what the agent says it did. A result
  that misdescribes its change is judged on the description.
- It is not a security control. Text in a result can try to steer the model;
  the rules never let an unsettled or steered answer reach green on its own
  say-so, but a careful reviewer is still the control.
- It does not decide. Reviews, approval gates and the land gate work the same
  with it on, off or failing.
