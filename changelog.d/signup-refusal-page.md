### A refused sign-up comes back to the form with a reason

- **Fixed:** when the human check, the workspace name or an invite refused a browser's **Create your workspace**, the browser was left on a raw JSON error, and the check it had solved was spent. It now goes back to the form, with the name it typed kept and one plain sentence saying what to do. An API caller still gets the problem document it got before.
