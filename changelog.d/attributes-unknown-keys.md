### Task attributes refuse an unknown key

- **Fixed:** `PUT /threads/{id}/attributes`, and `attributes` on a thread create or a plan child, return 400 on a key they do not know instead of dropping it. A body wrapped as `{"attributes": {…}}` used to parse as all defaults and wipe the thread's engine, acceptance and Linear issue. The MCP tools were already strict; the `create_thread` schema now says so (`additionalProperties: false`).
