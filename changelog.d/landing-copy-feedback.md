### Landing page Copy buttons work on plain http and say what happened

- **Fixed:** The landing page's Copy buttons did nothing on a self-hosted page served over plain http (a LAN address), where browsers hide the Clipboard API. They now fall back to copying from a selected text field, announce "Copied" to screen readers, and show "Select and copy" when the browser refuses both ways.
