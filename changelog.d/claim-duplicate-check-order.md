### A re-claim of an assigned duplicate gets the usual answer

- **Fixed:** Claiming a thread that is already assigned now returns `claimed: false` even when the thread is also a live duplicate, instead of the duplicate refusal. The holder re-claiming gets the usual answer; an unassigned duplicate is still refused (#1580).
