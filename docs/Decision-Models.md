# Advice from a decision model

When a task is handed to review, Maidan can give the reviewer a second read of
the hand-off: a decision model answers ten small questions about it, and the
reviewer sees what it found before opening the diff. The model is
[TypeSafe](https://typesafe.ai)'s Jev, a System One model: it answers typed
questions with calibrated probabilities instead of writing text.

The design follows TypeSafe's own advice for a model like this: judgments
from the model, decisions in code. Maidan does not ask whether the work should
land. It asks ten yes/no questions, each about one thing a person could point
to in the hand-off, and a table in the server turns the ten probabilities into
green, amber or red. The questions and the table are both in
`crates/maidan-server/src/land_gate_advisor.rs`, and nowhere else.

The result is advice for whoever reviews. Reviews, approval gates and the land
gate work the same with it on, off or failing.

It is off unless the instance turns it on, and it needs a TypeSafe API key:
the instance's, or the workspace's own.

## What the model reads

The server builds the packet from the thread, so every call sends the same
shape and a caller cannot choose what leaves:

```json
{
  "task": {
    "title": "Rate-limit the public API",
    "description": "60 requests a minute per token, then 429.",
    "acceptance": "cargo test passes; the 61st request in a minute gets 429."
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
  and with any string over 4,000 characters cut.
- `agent_messages` are the last ten messages on the thread from the member
  that produced the result, each cut at 1,500 characters, wherever they are
  in the thread. Nobody else's messages are sent, and the field is left out
  when there are none.

The whole request stays under 16 KiB, the questions included, so what one
call costs has a ceiling and a relay in front of the provider can hold the
same line. When a hand-off is larger, the agent's oldest messages are left out
first, then long strings in the result are cut shorter (2,000 characters,
then 1,000, 500, 250). A hand-off that is still too large after the last cut
is not sent: the call answers `409` and there is no advice.

The task's opening message (the thread's first message, when the member that
made the thread wrote it) is not sent, and that was measured. An opening
message tells the agent how to work (claim the task, say what you ran, send
it to review), and a hand-off cannot show those steps. With it in the packet
the model was no longer sure the hand-off did what the task asks, and on four
of sixteen labelled hand-offs the advice came out different from its label;
without it, on none. So say what a task asks for in its description and how
to check it in `acceptance`: MCP `create_thread` takes both.

That is everything that leaves the deployment, and it goes only to the host in
`MAIDAN_JEV_BASE_URL`.

## The ten judgments

Each is one question with a yes and a no described, sent together in one
request. The model answers each with the probability of yes. At 0.7 or above a
judgment reads as yes, at 0.3 or below as no, and in between it is unsettled:
the hand-off does not say enough for the model to tell. The two thresholds
are TypeSafe's suggested starting points, the same for every question, and
were not fitted to the labelled hand-offs below.

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

`done` is a judgment about what the hand-off says, not about whether the
work is good: the model has the agent's account and nothing else.

Two questions name two things each: `changes_access` asks about sign-in or
permissions, and `deletes_data` about deleting data or adding something that
deletes it. A question is usually better asked as two when it holds an "or",
so both were tried split in two. On all 36 labelled hand-offs the pair read
the same as the one question, and the rules treat both halves alike, so each
stays one row for the reviewer.

## The decision, in code

| Advice | When |
|---|---|
| **Red** | `unresolved_failure` is yes; or `done` is no and `beyond_task` is yes (the hand-off is about other work) |
| **Green** | `done` is yes; `unresolved_failure`, `left_undone`, `asks_person`, `beyond_task`, `changes_access`, `deletes_data` and `flags_risk` are all no; and either `code_change` is no or `tests_passed` is yes |
| **Amber** | everything else |

An unsettled judgment never counts toward green: a doubt about the work or
about any warning sign is amber, a task that may ask for a code change needs
its tests, and tests that may have passed are read as not reported. The only
unsettled answers a green can carry are the two that could not change it:
whether the task asks for code when its tests passed anyway, and whether tests
passed on a task that asks for no code.

A change to sign-in or permissions, a deletion of stored data and a warning
from the agent are amber by policy, not because the model disapproves: those
are the hand-offs a person should look at. The answer lists everything that kept it from green (`reasons`), with
what made it red first, and marks the judgments behind each reason.

| Reason | It means |
|---|---|
| `unresolved_failure` | Something is reported as still failing, or the model could not rule it out |
| `different_work` | `done` is no and `beyond_task` is yes: the hand-off is about something other than the task |
| `not_shown_done` | `done` is not a yes |
| `left_undone`, `asks_person`, `beyond_task`, `changes_access`, `deletes_data`, `flags_risk` | That judgment is not a settled no |
| `tests_not_reported` | The task asks, or may ask, for a code change, and `tests_passed` is not a yes |

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
  "usage": { "input_tokens": 1118, "output_tokens": 10 },
  "latency_ms": 148
}
```

Each judgment carries its `reading`: the answer as a statement ("Reports no
passing tests"), so a reader does not have to work out whether a yes is good.
One reading depends on the rules: when a task asks for no code change, missing
tests are no concern, and the row says so ("Reports no passing tests, which this
task does not call for"). The console shows how sure the model is of each
statement, which is its probability for the answer shown: a 1% chance of yes
is shown as the no, 99% sure.

An answer is held for fifteen minutes under the exact packet it was given
for and the key that paid for it, so a sheet opened twice asks once; a
changed result or a new message from the agent asks again, and
`{"refresh": true}` always does.

| Status | Meaning |
|---|---|
| 404 | The instance does not offer advice |
| 409 | The task has no result yet, the workspace has no key to ask with, or the hand-off is too large to send |
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
# Store this workspace's key (token:admin: the workspace's admin). The
# provider is asked to accept it first; a key it refuses is not stored.
curl -X PUT "$MAIDAN/workspaces/$WS/land-gate/advisor" \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"api_key": "…"}'
# What is set up: {"enabled": true, "key_source": "workspace", "provider": "typesafe", "model": "jev-latest", "host": "api.typesafe.ai"}
curl "$MAIDAN/workspaces/$WS/land-gate/advisor" -H "authorization: Bearer $TOKEN"
# Forget it.
curl -X DELETE "$MAIDAN/workspaces/$WS/land-gate/advisor" -H "authorization: Bearer $TOKEN"
```

The key is kept as a workspace secret, encrypted at rest (the instance needs
`FEDERATION_ENCRYPTION_KEY`), under a name the server keeps for itself
(`maidan:typesafe_api_key`). A colon is outside the names a member may give a
secret, so the generic secret routes and tools cannot create, list, resolve,
substitute or delete it: the only way in is this route, after the provider
accepted the key, and the only place it goes is the provider. In the console
it is under **Tools**, **Workspace**, for a person signed in who holds
`token:admin` or may mint the workspace's first admin token. An instance
that was set up without `FEDERATION_ENCRYPTION_KEY` has nowhere to keep a
workspace's key: the `PUT` answers `409`, and advice there uses the
instance's key.

On maidan.world a workspace brings its own key. try.maidan.world asks with the
demo's: when the page's scripted agent has sent **Say hello** to review, the
page asks for advice on that hand-off with the sandbox's own token
(`advise_land_gate`) and shows the answer, the ten judgments included, before
it sends the visitor to the console to approve. The task's sheet there shows
the same answer, held from that call. When the demo has no key, or the model
does not answer within four seconds (the server gives the model a second and
a half; the page allows for the round trip), the page shows nothing and goes
on. Where the
demo has a key, the page says before a sandbox exists that tasks sent to
review there are read by Jev (`advice` in `/try-config.json`).

## Turning it on

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_JEV_LAND_GATE_ENABLED` | `0` | `1` turns the routes and the tool on. An invalid value refuses boot. With it off, none of the others are read |
| `TYPESAFE_API_KEY` | none | The instance's own key, for workspaces that stored none. Leave it unset to have every workspace bring its own |
| `MAIDAN_JEV_MODEL` | `jev-latest` | A model or alias from TypeSafe's `GET /v1/models` |
| `MAIDAN_JEV_TIMEOUT_MS` | `1500` | Per request. A timeout fails only the advice call, with `502` |
| `MAIDAN_JEV_BASE_URL` | `https://api.typesafe.ai/` | An origin only, behind the same egress guard as other outbound calls. `https`, since the key travels as a bearer; plain `http` only for a relay of your own whose `.internal` name is listed in `MAIDAN_INTERNAL_EGRESS_HOSTS`. The name is resolved on the first call and again every ten minutes: a provider that cannot be reached fails the advice call, never the server |

## Measuring it

`scripts/eval-land-gate-advisor.py` runs labelled hand-offs through a running
Maidan and reports, for every judgment and for the advice, how many answers
agree with the labels. The labelled set it ships with,
`scripts/land-gate-advice-cases.json`, is 36 synthetic hand-offs across code
changes, documentation, investigation, support replies and infrastructure
work, written to include the hard cases: tests that were not run, a question
left in a message, work on the wrong thing, a result that tells the reader
how to answer.

The result on 2026-11-20, against `jev-1.14.0` through a running server: of
325 labelled answers, 314 agreed with their label, 11 were unsettled and none
was a settled wrong answer; the advice matched its label on 34 of the 36
hand-offs. The two it missed are worth knowing:

- A memory limit raised in a production values file, tested on staging, read
  as nothing flagged where we had labelled it worth a look. Every judgment
  was right. None of the ten questions asks whether a change touches
  production configuration, so the rules had nothing to go on.
- Release notes handed over with "let me know if you would like a longer
  version" read as worth a look: the model was unsure whether part of the
  task was left undone. We had labelled it nothing flagged.

We wrote these hand-offs and their labels ourselves, so this is a regression
set, not an independent test. Use your own hand-offs before you rely on it: a
model that reads our examples well has not been shown to read yours.

## Limits

- It reads what the agent says it did, not the diff. A result that
  misdescribes its change is judged on the description.
- It is not a security control. The agent writes the text the model reads,
  and text that steers the model into settled, favourable answers gets green
  advice: the rules read probabilities and cannot tell a steered answer from
  an honest one. The reviewer is the control, and the advice says where to
  look first.
