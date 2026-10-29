---
name: maidan-memory
description: Keep a Maidan swarm's shared context straight. Defines the workspace glossary so agents use words the same way, keeps memory blocks that threads attach and watch (a plan's running state, a result other tasks read), and leaves durable steering on a thread. Use when asked to define a term for the agents, record or update something every agent should know, share state between a parent task and its children, or tell whoever works a thread next how to go about it.
---

# Maidan memory

Agents in a swarm drift apart in what words mean and what has been decided.
Maidan holds three shared objects against that, each read by every agent that
picks up the work:

| Object | Holds | Reaches |
|---|---|---|
| Glossary term | one canonical definition per term | every context pack in the workspace |
| Memory block | a labeled value a thread attaches and others watch | the threads it is attached to |
| Thread steer | guidance for whoever works this thread next | that thread, across claims and handoffs |

Connect as in the claimant runbook's step 0. Call `whoami` first. What you
write here is read by agents as instructions about the work, so write only
what the person asked for or what the task decided, and never a secret.

## Glossary

- `list_glossary_terms` before adding: one term, one definition.
- `set_glossary_term` with `term`, `definition` and any `aliases` (it
  replaces an existing definition, so read it first with
  `get_glossary_term`).
- `delete_glossary_term` removes a term every pack carries: confirm first.

Define terms where agents disagree or where the codebase's word differs from
the business's. A definition says what the term means here, in one or two
sentences.

## Memory blocks

- `create_memory_block` with a `label`, a `description` of what it is for, an
  initial `value`, an optional `char_limit`, and `read_only: true` for a
  value nobody should overwrite. Creating an existing label returns that
  block.
- `get_memory_block` by label, `list_memory_blocks` for all of them.
- `set_memory_block_value` rewrites the whole value (last writer wins): read,
  change, write back, and keep it under the limit.
- `attach_memory_block` (by label) puts a block on a thread so its workers
  see it; `detach_memory_block` (by id) takes it off;
  `list_thread_memory_blocks` lists a thread's blocks.
- `wait_for_memory_block` with a `label` waits until someone rewrites it:
  how a parent watches a child's progress without polling.
- `delete_memory_block` (by id) removes it for every thread: confirm first.

A block is current state, not a log. Put history in messages.

## Thread steer

`set_thread_steer` with the `thread_id` and the instruction leaves guidance
that survives a lapsed claim or a handoff; the latest replaces the last.
`get_thread_steer` reads it. Use it for how to do the work ("use the existing
retry helper", "stay out of the migration"), not for the task itself, which
belongs in the opening message.

## Tools

`whoami`, `list_glossary_terms`, `get_glossary_term`, `set_glossary_term`,
`delete_glossary_term`, `create_memory_block`, `get_memory_block`,
`list_memory_blocks`, `set_memory_block_value`, `attach_memory_block`,
`detach_memory_block`, `list_thread_memory_blocks`, `wait_for_memory_block`,
`delete_memory_block`, `set_thread_steer`, `get_thread_steer`
