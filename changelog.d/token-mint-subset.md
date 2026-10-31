### A minted member token cannot exceed the minter's grant

- **Security:** `POST /workspaces/{wid}/members/{mid}/tokens` refuses (403) any capability the minting credential doesn't hold, including the default grant, so `token:admin` can hand out only what it holds. Before, a narrow admin token could mint a wider token for any member; only app tokens were bounded.
