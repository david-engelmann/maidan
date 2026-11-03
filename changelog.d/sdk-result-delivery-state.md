### The SDKs read a result's delivery state

- **Added:** the four SDKs model `delivery_state` and `deliveries` on `getResult` / `get_result` / `GetResult` (from #1909), so a claimant sees whether its result reached its `deliver_to` targets without a second request. The SDK interop check failed on dev because the server sent fields the SDKs did not declare.
- **Fixed:** the deploy pins check skips releases that were tagged but never published (`scripts/unpublished-releases`: 418.0.0 and 420.0.0), which have no images to pin; before, an unpublished tag demanded pins on images that do not exist.
