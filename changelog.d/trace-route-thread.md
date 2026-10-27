### Request spans name their route and thread

- **Added:** every request's `inbound` span carries `http.route` (the route template, never the path), `http.request.method`, and `maidan.thread_id` when the route or an MCP tool call's `thread_id` argument names a thread. A caller that sends `traceparent` on its calls for a thread (the swarm's pi harness uses the thread id as its trace id) gets one trace across both services, and Tempo can find every request made for a thread (ENG-6218).
