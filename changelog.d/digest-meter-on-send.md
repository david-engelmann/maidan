### A sent digest is always metered and never re-sent by a failed save

- **Fixed:** the digest sweeper now advances a member's watermark before sending and puts it back if the send fails. A failed save sends nothing, so a sent digest is always counted as a delivery and is no longer re-sent next tick because the watermark could not be saved (#1721).
