### A result delivery that gives up alerts the operator channel

- **Added:** When a task result's delivery exhausts its retries, the operator channel now gets an alert naming the thread, the target (for GitHub, the repository and pull request), the error, and the replay route, alongside the existing `result_delivery_failed` event (#1649).
