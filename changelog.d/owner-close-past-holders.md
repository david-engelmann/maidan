### The owner-close check counts past holders

- **Fixed:** On an owner-governed thread, a member who held the claim earlier can no longer land (close) it after the claim was released or reaped. The separation-of-duties check now reads the thread's durable holder record, not only the live assignee (#1569).
