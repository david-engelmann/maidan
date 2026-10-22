### A claimant reads its task's attributes where it reads its work

- **Fixed:** the thread context pack (`thread.attributes`) and the `claim_next` answer, on REST and MCP, now carry the task's attributes (`kind`, `engine`, `size`, `acceptance`, `linear_issue`), as docs/Swarms.md said they did. Before, only `GET /threads/{id}` had them. The claim's pin is still the hash of the claimed row.
