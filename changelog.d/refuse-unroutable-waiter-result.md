### A task result that could never be delivered is refused instead of stored

- **Fixed:** `PUT /threads/{id}/result` (400) and the `set_thread_result` MCP tool (invalid params) refuse a `maidan.waiter.result/1` envelope with no string `result_kind` or `status`, naming the missing field. Result routing drops such an envelope, so it used to be stored and silently never delivered (#1649).
