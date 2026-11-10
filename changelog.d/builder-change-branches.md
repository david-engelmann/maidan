### Build seats deliver on `feature/builder-` branches

- **Added:** the change flow writes `feature/builder-[a-z0-9][a-z0-9-]*` branches as well as `feature/agent-…`, under the same rules: never a protected branch, never `prod` as a base, never another change's branch as a base, either prefix in any case. A repository's gate can hold a build seat's changes to different checks from an unattended producer's by branch name alone.
