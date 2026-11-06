// happy-dom has no matchMedia; motion and the console ask for reduced motion.
if (typeof window.matchMedia !== "function") {
  window.matchMedia = (query: string) => ({
    matches: query.includes("prefers-reduced-motion"),
    media: query,
    onchange: null,
    addListener() {},
    removeListener() {},
    addEventListener() {},
    removeEventListener() {},
    dispatchEvent() {
      return false;
    },
  });
}

if (typeof CSS === "undefined" || typeof CSS.escape !== "function") {
  const escape = (value: string) => value.replace(/([^a-zA-Z0-9_-])/g, "\\$1");
  globalThis.CSS = { ...globalThis.CSS, escape } as typeof CSS;
}
