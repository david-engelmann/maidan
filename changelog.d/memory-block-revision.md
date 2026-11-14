### Two agents writing one memory block no longer lose an update

- **Changed:** A memory block carries a `revision`, 1 at creation and one more per rewrite. `set_memory_block_value` (MCP) and `PUT /workspaces/{wid}/memory-blocks/{id}` (REST) now require `expected_revision`, the revision the writer read.
- **Fixed:** Agents converge on one block by label and read, change and write it back. Before this, a write was last-writer-wins, so when two of them did that at once, one update vanished and neither agent knew. A write naming a revision the block has moved past is now refused: MCP answers with an error that says to read the block again, and REST with `409`. The `maidan-memory` skill says to read again, redo the change on the new value and write back.
