### A child becomes a review only at creation

- **Fixed:** Setting `kind = review` on an existing child thread is refused with 409, so relabelling an ordinary child no longer frees it from its parent's ordering hold. A child created as a review, and a thread without a parent, can still be set to `review` (#1556).
