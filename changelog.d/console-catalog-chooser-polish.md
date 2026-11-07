### The catalog filters tools by role and shows what a call really requires

- **Added:** the tool catalog filters by the Worker, Reviewer and Coordinator profiles (`/mcp/worker`, `/mcp/reviewer`, `/mcp/coordinator`) and by read-only and destructive hints. A tool's detail names the profile endpoints that list it, and its name can be copied.
- **Changed:** parameters that are required only as alternatives, such as a message's `body` or `content`, are labeled as one of required. Nullable enums, array-item enums, formats and defaults come from the schema instead of being shown as optional free text. The wide catalog pane uses the standard empty state when no tool is selected.
- **Fixed:** the workspace chooser closes when the workspace list cannot be loaded, keeps Tab inside the dialog, returns focus on close, and dismisses from the backdrop.
