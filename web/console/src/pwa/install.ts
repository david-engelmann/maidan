// Service worker registration and the install prompt, kept outside React so
// the beforeinstallprompt event is caught even before the Tools tab mounts.

type InstallPrompt = Event & { prompt(): Promise<void>; userChoice: Promise<{ outcome: string }> };
export type InstallState = { canPrompt: boolean; installed: boolean };

let deferred: InstallPrompt | null = null;
let state: InstallState = { canPrompt: false, installed: false };
const listeners = new Set<() => void>();
const set = (next: InstallState) => {
  state = next;
  for (const l of listeners) l();
};

export const installStore = {
  subscribe(fn: () => void): () => void {
    listeners.add(fn);
    return () => listeners.delete(fn);
  },
  getSnapshot: (): InstallState => state,
};

export function standalone(): boolean {
  return window.matchMedia?.("(display-mode: standalone)").matches || (navigator as Navigator & { standalone?: boolean }).standalone === true;
}

/** Register /console/sw.js and start listening for the install prompt. */
export function startPwa(): void {
  set({ canPrompt: false, installed: standalone() });
  window.addEventListener("beforeinstallprompt", (e) => {
    e.preventDefault();
    deferred = e as InstallPrompt;
    set({ ...state, canPrompt: true });
  });
  window.addEventListener("appinstalled", () => {
    deferred = null;
    set({ canPrompt: false, installed: true });
  });
  if ("serviceWorker" in navigator) {
    void navigator.serviceWorker.register("/console/sw.js", { scope: "/console/", type: "module" }).catch(() => undefined);
  }
}

/** Show the browser's install dialog; false when there is none to show. */
export async function promptInstall(): Promise<boolean> {
  if (!deferred) return false;
  const d = deferred;
  deferred = null;
  set({ ...state, canPrompt: false });
  await d.prompt();
  return (await d.userChoice).outcome === "accepted";
}

/** The push manager of the console's worker, or null when push is unavailable. */
export async function consolePushManager(): Promise<PushManager | null> {
  if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window)) return null;
  const reg = (await navigator.serviceWorker.getRegistration("/console/")) ?? (await navigator.serviceWorker.register("/console/sw.js", { scope: "/console/", type: "module" }));
  return reg.pushManager;
}
