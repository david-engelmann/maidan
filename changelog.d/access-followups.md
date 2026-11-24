### Share tickets, buried decisions and run occupancy keep to what the caller may read

- **Fixed:** A share ticket can no longer be minted for the shared direct-message channel, and a ticket for a private channel needs a creator who is a member of that channel. A ticket for the direct-message channel that was minted earlier serves nothing.
- **Fixed:** `list_buried_decisions`, its REST route and the digest email no longer return a result from a thread the member cannot read now, such as one in a private channel the member was removed from or a direct message the member is not part of.
- **Fixed:** `get_run_occupancy` and its REST route count only the threads the caller can read, so a private channel or a direct message adds nothing to a run's counts for someone outside it.
