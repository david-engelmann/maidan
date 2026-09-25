# Cluster 418 retro — the adoption surface

> Post-gate hardening · source record (no `v418.0.0` tag yet; the tag is the maintainer's) · PRs #1032, #1036, #1040, #1046, #1047, #1048, #1049, #1050 + close record

## Outcome

Someone arriving at Maidan can now start a working agent with one command and
trust that the examples ran. Every example has a last-verified date and the
versions it ran against. Two compose recipes (a coding agent, and a deploy
that waits on a person) boot a server from the checkout and install their
agent as an app with a scoped token. A Goose recipe works tasks through the
same loop. The `/ui` composer takes a pasted file as an artifact.
CONTRIBUTING says how work is claimed and what the project does not promise.

| Slice | PR | Result |
|-------|----|--------|
| 418.1 (#48) | #1032 | CONTRIBUTING handbook-lite: claiming work, security timelines taken from SECURITY.md, ownership, claims we do not make, how a release is cut. |
| #49 | #1040 | A file pasted into the `/ui` composer uploads as an artifact and attaches to the selected thread; the client filename is never used. |
| #47 | #1036 | Every example run and recorded; `a2a_interop.py` failed against an auth-on server and now works inside the token's workspace. |
| #47 | #1047 | `examples/recipes/`: a coding agent on the REST waiter loop, and an approval-gated deploy over MCP. |
| fix | #1046 | `claim_next_thread` hands out only `open` threads. |
| docs | #1048 | Integration step 3: an agent joins as an installed app. |
| docs | #1049 | The framework examples' hero loop gains `transition_thread`. |
| #50 | #1050 | `examples/goose/maidan-waiter.yaml`, a Goose recipe for the waiter loop. |

## Decisions

- **Recipes build Maidan from the checkout, not from a release.** The last
  release (v410) predates #1000 and still requires `author_id` on a message,
  which `main` now refuses. An example on `main` has to speak `main`'s API.
- **An agent joins as an app.** With auth on, member creation is bootstrap-only
  and SCIM provisions people. Installing an app creates the agent's member,
  and the installation caps every token it will hold. The recipes and
  Integration.md now both teach this.
- **Finished work goes to review before it is released.** The waiter loop sets
  its result, calls `start_review`, then releases. Closing stays with someone
  else.
- **An example that did not run says so.** The Goose recipe's row reads "not
  run inside Goose"; what was verified is Maidan's side, replayed with a token
  holding exactly the recipe's capabilities.

## What surprised us

- **Running an example found a store bug no test had.** The coding-agent
  recipe did its one task, released it, claimed it again, and repeated until it
  was rate-limited. `claim_next` excluded blocked, gated, parked,
  skill-mismatched and frozen threads, but not finished ones. Queue depth and
  occupancy, which say they mirror the claim predicate, counted `in_review`
  threads as ready.
- **The onboarding page could not be followed with auth on.** Step 3 minted a
  token for a member the reader had no way to create.
- **A curated tool list encoded the old behaviour.** The "hero six" had no way
  to move a thread out of `open`, so it only worked because finished threads
  used to be claimable too.

## Method

Each example was run end to end against an auth-on server built from `main`,
and the run is recorded in `examples/README.md`. The claim fix was checked by
reverting it and requiring its test to fail on both backends.

## Carried forward

- Run the Goose recipe inside Goose once a provider key is available.
- An OpenHands claimant is not written.
- No way back from `in_review`: whether the FSM gets a `request_changes`
  reopen is a decision for the maintainer.
- A release cut from `main` would let the quickstart and the recipes share an
  image; tagging is the maintainer's call.
