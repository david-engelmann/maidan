### Mint a worker token on the Connect page

- **Added:** Connect offers one-click Cursor and VS Code install links that carry only the MCP URL, so the browser signs in with OAuth and no token is copied. Naming an agent on the same page mints a scoped `maidan.agent.worker` token once and fills in that client's setup, including one-click Cursor and VS Code links.
- **Changed:** Connect no longer installs Cursor with the placeholder `REPLACE_WITH_MAIDAN_TOKEN`. Someone who already has a token opens the reference snippets and replaces that placeholder.
