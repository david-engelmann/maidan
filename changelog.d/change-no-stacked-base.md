### A change no longer opens on another change's branch

- **Changed:** a `github_branch` target whose base is another change's `feature/agent-*` branch is now refused, with a reason that names `depends_on`. Stacked child pull requests were closed by GitHub when their base landed and was deleted (or when the merge queue closed it). Every change now opens on the repository's base, and the order between changes is a thread dependency, which holds the child unclaimed until its parent closes, so the child is built on a base that already has the parent.
