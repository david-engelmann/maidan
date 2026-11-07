### A task stops returning to the queue after three review rounds

- **Added:** A thread can be sent back from review only so many times before a person decides whether it gets another round. The cap is 3 unless a thread sets its own, including when it has no review requirement yet. `0` leaves the cap off. Tightening it needs `thread:transition`. Raising or removing it needs `channel:admin`. Accept returns the task to the queue for another cycle. Decline keeps it off the queue. A worker token cannot decide the gate or disarm the cap.
