import { DEMO_SERVER_ENV } from "./demo-config.js";

export const DEMO_PUBLIC_ORIGIN_VAR = "MAIDAN_PUBLIC_ORIGIN";
export const DEMO_DEFAULT_PUBLIC_ORIGIN = "https://try.maidan.world";

/**
 * The public origin the demo container advertises. Only an `https` origin
 * without credentials is accepted; anything else falls back to the try host
 * so a mis-set Worker var cannot mint `http:` or credential-bearing links.
 */
export function normalizePublicOrigin(origin) {
  const raw = typeof origin === "string" && origin.trim() ? origin.trim() : DEMO_DEFAULT_PUBLIC_ORIGIN;
  let url;
  try {
    url = new URL(raw);
  } catch {
    return DEMO_DEFAULT_PUBLIC_ORIGIN;
  }
  if (url.protocol !== "https:") return DEMO_DEFAULT_PUBLIC_ORIGIN;
  if (url.username || url.password) return DEMO_DEFAULT_PUBLIC_ORIGIN;
  if (!url.hostname) return DEMO_DEFAULT_PUBLIC_ORIGIN;
  return url.origin;
}

// The container's environment: the fixed demo settings plus the public origin.
// Nothing from the Worker's secrets is passed in, so the demo holds no
// production credential.
export function demoEnv(origin) {
  return { ...DEMO_SERVER_ENV, MAIDAN_PUBLIC_ORIGIN: normalizePublicOrigin(origin) };
}
