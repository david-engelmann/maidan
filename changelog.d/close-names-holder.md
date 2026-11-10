### Every close names the claim it ended; the withdrawal comment finds itself on any pull request

- **Fixed:** a merge that lands a task in its owner's name, and a plan that
  closes when its last child lands, now name the member whose claim the close
  ended (`previous_assignee_id` on `ThreadStateChanged`), so that worker and
  its followers are told, as they already were for a cancel, a close-abandoned
  and an ordinary close by someone else.
- **Fixed:** a withdrawal's retry looks for its own comment only among those
  written since the withdrawal was queued (GitHub's `since`), so a pull
  request with more than 1,000 older comments no longer gets the comment
  twice.
