### The stale token rotation UI test no longer hangs

- **Fixed:** the UI test for a token rotation that lands after sign-out could wait forever on a held `/rotate` request. Signing out navigates the page, which can drop the paused request without a `requestfailed` event. The test now reads the rotation's outcome inside its route handler and waits until the request is held before signing out.
