### The console header stays readable on a phone, and reduced motion stays off

- **Fixed:** At 390px the header shows Search as a word, the connection word when the console is not live, and a pasted or delegated token. Reduced motion now sets animation and transition to none and keeps scrolling instant, including a message jump and a Needs you row leaving.
- **Fixed:** Playwright layout checks wait until a control has a box, and the Needs you focus check waits until the row has redrawn, so a phone-width reflow cannot read a missing box or a focus that has not landed.
