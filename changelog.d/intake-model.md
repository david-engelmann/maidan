### Intake: proposed work before it is a task

- **Added:** An intake queue per workspace (`/workspaces/{wid}/intake`). Agents, humans and integrations propose items; a member accepts one into a new or existing thread, or rejects it with a required reason. Proposals and triage are announced as `intake_proposed`, `intake_accepted` and `intake_rejected`, and triage is audited.

A failed member lookup on propose is now an error instead of a human proposal; list and get show an accepted item's `thread_id` only to callers who can read that thread; intake events carry their actor (`member_id`) and `intake_accepted` its thread (`thread_id`) for filtered readers.
