### A phone-width audit waits for the button's box

- **Fixed:** The phone-width UI test read `boundingBox()` once, as soon as the Approve button was visible. A needs-you redraw, or the phone-width reflow, can report the button visible before it has a box, so the read was null. The test now waits until the button has a laid-out box and that box sits on the 390 px screen.
