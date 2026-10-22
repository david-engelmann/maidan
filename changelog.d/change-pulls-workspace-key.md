### Two workspaces can track the same pull request, each on its own row

- **Fixed:** tracked pull requests, their delivery rounds and their CI reads are keyed by workspace as well as repo and number (migration 0194, both backends). A second workspace watching or delivering to a pull request another one tracks now gets its own thread and notices instead of being refused. Closing, settling or recording an outcome touches only the caller's row.
- **Security:** only the workspace that delivered a pull request's head may rebase it, push to it or enqueue it.
