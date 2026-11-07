### A long seat run keeps its claim

- **Fixed:** The seat runner renews its claim every third of `lease_secs` while the harness runs, and stops renewing before it releases. A run longer than the lease no longer loses its claim to another seat mid-work; a refused renewal lets the harness finish and the release report it.
