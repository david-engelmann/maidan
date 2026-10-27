### A change's base must be a short branch name

- **Fixed:** The change flow now refuses a base written as a qualified ref (`refs/heads/…` or `heads/…`), in both the change target and the mark-ready base map; a qualified ref slipped past the checks that refuse `prod` and another change's branch as a base (#1570).
