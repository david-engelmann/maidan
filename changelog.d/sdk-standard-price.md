### SDK usage normalizers carry the standard price

- **Added:** The four SDKs' usage normalizers take a `batch` flag and a `standard_price` snapshot and put them in the evidence, so a batch or flex call reports what its lane saved without hand-building the evidence (#1592).
- **Added:** Shared fixtures for a flex call and a batch call with a standard price, and an invalid one for a standard price on a standard-tier call. The SDKs refuse it with the server's message, and the ledger's fixture test checks the server refuses it too.
