// Runs before the stylesheet and the first paint, as a plain external script:
// the console's CSP admits no inline script, and the module bundle runs too
// late to stop a stored Light choice flashing the dark palette. The rule is
// src/core/theme.ts's, and test/theme.test.ts holds the two to each other.
(function () {
  var choice = "dark";
  try {
    var stored = window.localStorage.getItem("maidan_console_theme");
    if (stored === "light" || stored === "system") choice = stored;
  } catch (e) {
    // Storage blocked: the default stands.
  }
  var light = choice === "light";
  if (choice === "system") {
    try {
      light = window.matchMedia("(prefers-color-scheme: light)").matches;
    } catch (e) {
      light = false;
    }
  }
  var theme = light ? "light" : "dark";
  var root = document.documentElement;
  root.setAttribute("data-theme", theme);
  root.setAttribute("data-theme-choice", choice);
  var meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.setAttribute("content", light ? "#f2eee6" : "#0c0d0f");
})();
