### Deliveries cutoff for federation peer polling

- **Fixed:** An open sign-up workspace past its deliveries allowance kept polling the federation peers it created, sending outbound HTTP every tick. Its peers are now skipped while it is cut off, with the sync cursor left in place, and each peer poll counts one delivery.
