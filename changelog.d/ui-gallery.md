### A review gallery for every UI change

- **Added:** `npm run gallery` in `ui-tests/` captures the landing and the console's first screens at desktop and phone widths, and fails on a screen that scrolls sideways or a page that throws. The `ui gallery` CI job runs it on every pull request that changes code and uploads the pictures as the `ui-gallery` artifact.
