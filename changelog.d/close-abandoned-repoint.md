### Dependents move to the thread that replaces a duplicate or superseded close

- **Changed:** Closing a thread as `duplicate` or `superseded` with `duplicate_of` moves each open dependent onto `duplicate_of`, so it becomes ready when that thread lands, and posts a notice on the dependent. The replacement's own edge to the closed thread is dropped. A move that would make a cycle is skipped and the dependent is told. The audit row lists `repointed_dependents` and `kept_dependents`. An abandoned close leaves dependents waiting, as before (#1754).
