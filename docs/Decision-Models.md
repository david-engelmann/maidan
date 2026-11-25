# Advice from a decision model

When a task is handed to review, Maidan can give the reviewer a second read of
the hand-off: a decision model answers ten small questions about it, and the
reviewer sees what it found before opening the diff. A decision model answers
typed questions with probabilities instead of writing text. The server can
ask five: [TypeSafe](https://typesafe.ai)'s Jev, OpenAI's Decisions API,
Google's Gemini Decisions API, Perplexity's Decisions API and Cloudflare's
Clef on Workers AI. All five are asked the same ten questions and read by the
same rules, and a workspace chooses which by the keys it adds.

The design follows TypeSafe's own advice for a model like this: judgments
from the model, decisions in code. Maidan does not ask whether the work should
land. It asks ten yes/no questions, each about one thing a person could point
to in the hand-off, and a table in the server turns the ten probabilities into
green, amber or red. The questions and the table are both in
`crates/maidan-server/src/land_gate_advisor.rs`, and nowhere else.

The result is advice for whoever reviews, agent or person. Reviews and
approval gates work the same with it on, off or failing.

It is off unless the instance turns it on, and it needs an API key for at
least one provider: the instance's, or the workspace's own.

## The providers

| Provider | `provider` | Asked at | Default model | Instance key | A workspace's key is kept as |
|---|---|---|---|---|---|
| TypeSafe Jev | `typesafe` | `POST https://api.typesafe.ai/v1/systemone` | `jev-latest` | `TYPESAFE_API_KEY` | `maidan:typesafe_api_key` |
| OpenAI Decisions | `openai` | `POST https://api.openai.com/v1/decisions` | `gpt-6-luna` | `OPENAI_API_KEY` | `maidan:openai_api_key` |
| Gemini Decisions | `google` | `POST https://generativelanguage.googleapis.com/v1beta/decisions` | `gemini-decision-1` | `GEMINI_API_KEY` | `maidan:gemini_api_key` |
| Perplexity Decider | `perplexity` | `POST https://api.perplexity.ai/v1/decisions` | `pplx-decider-latest` | `PERPLEXITY_API_KEY` | `maidan:perplexity_api_key` |
| Cloudflare Clef | `cloudflare` | `POST https://api.cloudflare.com/client/v4/accounts/{account}/ai/run/@cf/cloudflare/clef` | `clef` | `CLOUDFLARE_WORKERS_AI_TOKEN` with `CLOUDFLARE_ACCOUNT_ID` | `maidan:cloudflare_workers_ai_key` |

A call may name its provider. One that names none asks the first provider
the workspace has a key for, in the order above. The questions' wording is
the same for all five, and each is sent in the provider's own shape:

- **TypeSafe** takes the packet as `state` and each question as a `noul`
  with what yes and no mean in its `criteria`.
- **OpenAI** takes the packet as a JSON part of one user message and each
  question as a `predicate`. A predicate has no field for what yes and no
  mean, so they follow the question in its `instructions`. OpenAI may decline
  a single question (`refusal`); that judgment is read as unsettled, which
  never counts toward green.
- **Google** takes the packet as text (its JSON) and each question as a
  `noul` with `true_description` and `false_description`. Its probabilities
  stop at 0.01 and 0.99. A request it blocks for safety fails as a whole, and
  the advice call fails with it. The API is `v1beta`. On Google's free tier
  what is sent may be used to improve its products; a paid key is the one to
  use for real work.
- **Perplexity** takes TypeSafe's request as it is: the packet as `state` and
  each question as a `noul` with `criteria`. `pplx-decider-flash-latest` is
  its faster model (`MAIDAN_PERPLEXITY_DECISIONS_MODEL`).
- **Cloudflare** takes the same request too, at an address that names the
  account and the model, and answers inside its API's envelope (`result`).
  Its key is two things, a token that may run Workers AI models and the id of
  the account it calls, so a workspace stores both (see "Keys") and an
  instance sets both. The account id is held to its shape, 32 hexadecimal
  characters, before it goes into the address. `clef-flash` is the faster
  model (`MAIDAN_CLEF_MODEL`). The server reads `CLOUDFLARE_WORKERS_AI_TOKEN`
  and never `CLOUDFLARE_API_TOKEN`: a token a host holds for deploys does not
  turn a provider on.

Each provider's answer is kept apart: an answer is held for fifteen minutes
under the provider that gave it, and one provider's answer is never shown as
another's.

The thresholds below were TypeSafe's suggested starting points and assume
probabilities that mean what they say. Whether a provider's 0.9 is right nine
times in ten is that provider's claim. Run the measurement under "Measuring
it" with each key before you rely on one.

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

That is everything that leaves the deployment, and it goes only to the
provider asked: the host in that provider's base URL variable.

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

A reason can rest on a judgment the model could not settle. The reason is the
same, and the `answer` of the judgment behind it is `unsettled`. The console
reads that and says so in the summary ("It is unclear whether it changes
sign-in or permissions") and keeps the plain sentence ("It changes sign-in or
permissions") for a settled yes. `not_shown_done` reads the same either way.

## Asking

```sh
curl -X POST "$MAIDAN/threads/$THREAD/land-gate/advice" \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' -d '{}'
```

MCP: `advise_land_gate` with `thread_id`. Both need `thread:transition` and a
thread the caller can read. Both take an optional `provider` (`typesafe`,
`openai`, `google`, `perplexity` or `cloudflare`), and the answer names the
provider and model that gave it:

```sh
curl -X POST "$MAIDAN/threads/$THREAD/land-gate/advice" \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"provider": "openai"}'
```

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

Each provider has its own key. A call to a provider uses the workspace's own
key for it when the workspace stored one, and the instance's otherwise. With
neither, that provider is refused.

```sh
# Store this workspace's key for a provider (token:admin: the workspace's
# admin). `provider` is typesafe when left out. The provider is asked to
# accept the key first; a key it refuses is not stored.
curl -X PUT "$MAIDAN/workspaces/$WS/land-gate/advisor" \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"api_key": "…", "provider": "openai"}'
# Cloudflare's key is a token and the account it calls. The check asks the
# model one small question of its own, since no read shows a token may run it.
curl -X PUT "$MAIDAN/workspaces/$WS/land-gate/advisor" \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"api_key": "…", "provider": "cloudflare", "account_id": "…"}'
# What is set up. The top-level fields describe the provider a call that
# names none would ask, and `providers` lists all five. `can_store_key` is
# false on an instance without FEDERATION_ENCRYPTION_KEY, which cannot keep a
# workspace's key (a PUT answers 409), and the console says so in place of the forms:
# {"enabled": true, "can_store_key": true, "key_source": "workspace", "provider": "openai", "model": "gpt-6-luna", "host": "api.openai.com",
#  "providers": [{"id": "typesafe", "name": "TypeSafe Jev", "model": "jev-latest", "host": "api.typesafe.ai"},
#                {"id": "openai", "name": "OpenAI Decisions", "model": "gpt-6-luna", "host": "api.openai.com", "key_source": "workspace"},
#                {"id": "google", "name": "Gemini Decisions", "model": "gemini-decision-1", "host": "generativelanguage.googleapis.com"},
#                {"id": "perplexity", "name": "Perplexity Decider", "model": "pplx-decider-latest", "host": "api.perplexity.ai"},
#                {"id": "cloudflare", "name": "Cloudflare Clef", "model": "clef", "host": "api.cloudflare.com"}]}
curl "$MAIDAN/workspaces/$WS/land-gate/advisor" -H "authorization: Bearer $TOKEN"
# Forget one provider's key.
curl -X DELETE "$MAIDAN/workspaces/$WS/land-gate/advisor?provider=openai" -H "authorization: Bearer $TOKEN"
```

A key is kept as a workspace secret, encrypted at rest (the instance needs
`FEDERATION_ENCRYPTION_KEY`), under a name the server keeps for itself (the
last column of the table above). A colon is outside the names a member may give a
secret, so the generic secret routes and tools cannot create, list, resolve,
substitute or delete it: the only way in is this route, after the provider
accepted the key, and the only place it goes is that provider. In the console
the keys are under **Tools**, **Workspace**, one row per provider, for a
person signed in who holds
`token:admin` or may mint the workspace's first admin token. An instance
that was set up without `FEDERATION_ENCRYPTION_KEY` has nowhere to keep a
workspace's key: the `PUT` answers `409`, and advice there uses the
instance's key.

With keys for more than one provider, the advice on a task has a switch: a
reviewer can ask another model about the same hand-off, and the console
remembers the choice.

On maidan.world a workspace brings its own keys. try.maidan.world asks with the
demo's, for each provider the demo holds a key for (Cloudflare's Clef is not
one the demo offers), and the page has the same switch: when the page's scripted agent has sent **Say hello** to review, the
page asks for advice on that hand-off with the sandbox's own token
(`advise_land_gate`) and shows the answer, the ten judgments included, before
it sends the visitor to the console to approve. The task's sheet there shows
the same answer, held from that call. When the demo has no key, or the model
does not answer within four seconds (the server gives the model a second and
a half, and once more if that ran out; the page allows for the round trip),
the page shows nothing and goes on. Where the
demo has a key, the page says before a sandbox exists that tasks sent to
review there are read by a decision model outside the demo (`advice` and
`providers` in `/try-config.json`).

## Turning it on

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_JEV_LAND_GATE_ENABLED` | `0` | `1` turns the routes and the tool on. An invalid value refuses boot. With it off, none of the others are read |
| `FEDERATION_ENCRYPTION_KEY` | none | Needed for a workspace to store a key of its own (see "Keys"). Without it only the instance's keys can be used |
| `TYPESAFE_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `PERPLEXITY_API_KEY` | none | The instance's own key for each provider, for workspaces that stored none. Leave one unset to have every workspace bring its own for that provider |
| `CLOUDFLARE_WORKERS_AI_TOKEN` with `CLOUDFLARE_ACCOUNT_ID` | none | The instance's own token for Cloudflare's Clef and the account it calls. The token without the account refuses boot; the account alone turns nothing on |
| `MAIDAN_JEV_MODEL` | `jev-latest` | A model or alias from TypeSafe's `GET /v1/models` |
| `MAIDAN_OPENAI_DECISIONS_MODEL` | `gpt-6-luna` | The model OpenAI's Decisions API is asked for |
| `MAIDAN_GEMINI_DECISIONS_MODEL` | `gemini-decision-1` | The model Google's Decisions API is asked for |
| `MAIDAN_PERPLEXITY_DECISIONS_MODEL` | `pplx-decider-latest` | The model Perplexity's Decisions API is asked for |
| `MAIDAN_CLEF_MODEL` | `clef` | The Workers AI model's short name, `clef` or `clef-flash` |
| `MAIDAN_JEV_TIMEOUT_MS` | `1500` | Per request, for every provider. A request that times out is made once more, so a call can take twice this before it fails; a second timeout fails only the advice call, with `502` |
| `MAIDAN_JEV_BASE_URL`, `MAIDAN_OPENAI_DECISIONS_BASE_URL`, `MAIDAN_GEMINI_DECISIONS_BASE_URL`, `MAIDAN_PERPLEXITY_DECISIONS_BASE_URL`, `MAIDAN_CLEF_BASE_URL` | `https://api.typesafe.ai/`, `https://api.openai.com/`, `https://generativelanguage.googleapis.com/`, `https://api.perplexity.ai/`, `https://api.cloudflare.com/` | Each an origin only, behind the same egress guard as other outbound calls. `https`, since the key travels in a header; plain `http` only for a relay of your own whose `.internal` name is listed in `MAIDAN_INTERNAL_EGRESS_HOSTS`, or for a stand-in on the same machine, a loopback address listed in `MAIDAN_PRIVATE_EGRESS_HOSTS`. The name is resolved on the first call and again every ten minutes: a provider that cannot be reached fails the advice call, never the server |

## Measuring it

`scripts/eval-land-gate-advisor.py` runs labelled hand-offs through a running
Maidan and reports, for every judgment and for the advice, how many answers
agree with the labels. The labelled set it ships with,
`scripts/land-gate-advice-cases.json`, is 36 synthetic hand-offs across code
changes, documentation, investigation, support replies and infrastructure
work, written to include the hard cases: tests that were not run, a question
left in a message, work on the wrong thing, a result that tells the reader
how to answer.

The script makes one task per hand-off in a channel of its own, and cancels
each task once its advice has been read.

The result on 2026-11-20, against `jev-1.14.0` through a running server: of
325 labelled answers, 314 agreed with their label, 11 were unsettled and none
was a settled wrong answer; the advice matched its label on 34 of the 36
hand-offs. Two more runs the same day read 315 and 10, then 313 and 12, with
none wrong and the same two misses: an answer near a threshold can settle
either way between runs. The two it missed are worth knowing:

- A memory limit raised in a production values file, tested on staging, read
  as nothing flagged where we had labelled it worth a look. Every judgment
  was right. None of the ten questions asks whether a change touches
  production configuration, so the rules had nothing to go on.
- Release notes handed over with "let me know if you would like a longer
  version" read as worth a look: the model was unsure whether part of the
  task was left undone. We had labelled it nothing flagged.

### Across the providers

One run each on 2026-11-21, the same 36 hand-offs through a locally built
server (`scripts/eval-land-gate-advisor.py --provider`):

| Provider | Model | Agreed | Unsettled | Wrong | Advice on its label | Median answer | Input tokens, all 36 |
|---|---|---|---|---|---|---|---|
| TypeSafe | `jev-1.14.0` | 313 | 12 | 0 | 34 of 36 | 78 ms | 41,366 |
| OpenAI | `gpt-6-luna` | 313 | 9 | 3 | 31 of 36 | 216 ms | 75,935 |
| Google | `gemini-decision-1` | 318 | 6 | 1 | 33 of 36 | 332 ms | 45,994 |
| Perplexity | `pplx-decider-latest` | 318 | 5 | 2 | 33 of 36 | 200 ms | 29,262 |
| Cloudflare | `clef` | 314 | 10 | 1 | 32 of 36 | 290 ms | 45,056 |

Read it with two things in mind. The questions were written and probed
against Jev, and the two thresholds are TypeSafe's, so Jev has the home
advantage here. And it is one run of a set we labelled ourselves.

- **A settled wrong answer** is a probability at or past a threshold on the
  wrong side. Jev gave none. OpenAI gave three: no passing tests where the
  hand-off reports them (0.01), a risk flagged where none was (0.77), and
  passing tests where none were reported (0.78). Google gave one: work beyond
  the task where there was none (0.76).
  Perplexity gave two: a change to sign-in or permissions where there was
  none (0.72), and no passing tests where the hand-off reports them (0.27).
  Cloudflare gave one: the task done where the work was on another endpoint
  (0.83).
- **All five** read the production values change as nothing flagged, for the
  reason above: no question asks about it.
- **OpenAI and Perplexity** also read a change to payment code as nothing
  flagged where we had labelled it worth a look. That is the direction to
  care about: advice that says nothing where a reviewer should look.
  **Cloudflare** did the same on a fix whose whole result is "Tests pass."
- **OpenAI, Google and Perplexity** read an agent blocked on access as worth
  a look where we had labelled it a reported problem.
- **Cloudflare** read two clean hand-offs as worth a look: an investigation
  that ends in a recommendation (a risk, it said), and the release notes
  with the offer of a longer version that Jev also stopped at.
- OpenAI's requests are larger because a predicate has no field for what yes
  and no mean, so both are sent in every question's instructions.
  Perplexity's count fewest tokens for the same text.

Four of the five never took longer than 0.8 seconds on any hand-off,
against the default 1.5 second timeout. Perplexity is the exception. Its
usual answer is as quick as the others, and about one in sixty is not: asked
the same small question 360 times in a row, `pplx-decider-latest` took five
to ten seconds for six of them, each on its own with quick answers on either
side. Three runs of this measurement stopped on one. So the server makes a
request that timed out once more before the advice call fails
(`MAIDAN_JEV_TIMEOUT_MS`). The row above is a run at the default timeout
through a server that asks again, in which no answer took longer than 0.7.

Perplexity and Cloudflare each have a faster model, measured the same day
through a relay in front of the server as it was before it knew them (both
take TypeSafe's request unchanged). `pplx-decider-flash-latest` agreed on 312
with 12 unsettled and 1 wrong, its advice on its label for 32 of 36, and none
of 60 answers took longer than 0.3 seconds. `clef-flash` agreed on 303 with
21 unsettled and 1 wrong, for 31 of 36.

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
  look first. How often a decision model is steered is measured by
  [Peira](https://peiratrial.dev), a benchmark from the same author: it
  gives a model the same decision twice, once clean and once with an attack
  in the input, and counts how often a right answer flips. All five
  providers here have results on its board (read 2026-11-21), and every
  model that ran its full suite flips some of its right answers under
  attack.
