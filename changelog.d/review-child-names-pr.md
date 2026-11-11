### A review child names the pull request and head it reviews

- **Changed:** a second-family review child spawned for a delivered change is titled `Second-family review: <repo>#<N> at <short sha>`, and its description reads `Pull request: <repo>#<N>` and `Head to review: <full sha>`, from the change's latest delivered round. A reviewer no longer needs a second read of the parent's deliveries to know what to review. A change the flow never delivered keeps the plain title.

- **Fixed:** on Activity, a short channel link such as `#build` shrank below the 44px touch target at phone width. A rule letting row links shrink outranked the link's own `min-width`. The Playwright phone check caught it once seeded channels had short names.
