### The console passes the React Compiler's checks

- **Changed:** `eslint-plugin-react-hooks` moves to 7.1.1, and the console no longer sets state from an effect where it can derive it, reads refs from closures built during render, assigns `location` from a memo, or calls `Date.now()` while rendering.
- **Fixed:** In Needs you, retrying a review's evidence ignores a late answer from the earlier try, and an answer for one task can no longer land on another.
- **Fixed:** A conversation left open overnight turns its "Today" separator into "Yesterday" at midnight, not at the next message.
- **Fixed:** The command palette shows a workspace's saved channels and people in the first render after a switch, and a new search query shows "Searching…" without a stale result render first.
