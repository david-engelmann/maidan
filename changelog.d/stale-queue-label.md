### A pull request the merge queue dropped takes its next round

- **Fixed:** When GitHub's merge queue drops a pull request (a merge conflict, failed checks) and leaves the queue's label on it, the next round's delivery takes the stale label off through the dequeue guard path and is then delivered. Before, it was refused for good. The new head is not queued unreviewed, and the next `POST /operator/github/enqueue` labels it again. Clearing the label is audited as `github.enqueue` with outcome `stale_label_cleared`.
