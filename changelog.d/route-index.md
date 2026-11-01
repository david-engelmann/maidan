### A route index at `/` when sign-up is off

- **Added:** Without sign-up, `GET /` returns a route index instead of a 404: `/openapi.json`, the MCP endpoints, `/health`, `/llms.txt` and the console at `/ui`. It's JSON by default, or a plain HTML list with no script or style, under a CSP that allows neither, when the client asks for HTML and not JSON. The waitlist page is unchanged when sign-up is on.
- **Changed:** A 404 problem's `detail` now ends with "GET /openapi.json lists the routes".
