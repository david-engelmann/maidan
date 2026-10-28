### A share ticket test does not expire before it is minted

- **Fixed:** The share-ticket test minted a ticket 50 ms after an instant taken before the insert. Create judges the deadline against its own later clock, so a slow runner minted a ticket that was already expired and the insert was refused. The test now uses a deadline a stalled runner cannot reach, and still checks that the ticket resolves until that exact instant.
