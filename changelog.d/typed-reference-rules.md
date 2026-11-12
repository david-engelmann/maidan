### Typed references are refused when they break the room's rules

- **Added:** `add_reference` now refuses a self-link, a cross-workspace
  link, a link whose source or target does not exist, and a cycle along
  `depends`, `supersedes`, or `duplicates`. Each refusal is
  `InvalidInput` (REST 400 / MCP InvalidParams). An identical edge is
  still a 409. Free-form `Other` relations and mixed thread/message
  pairs stay allowed.
