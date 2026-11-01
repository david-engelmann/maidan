import { DEMO_SERVER_ENV } from "./demo-config.js";

export const DEMO_PUBLIC_ORIGIN_VAR = "MAIDAN_PUBLIC_ORIGIN";

// The container's environment: the fixed demo settings plus the public origin.
// Nothing from the Worker's secrets is passed in, so the demo holds no
// production credential.
export function demoEnv(origin) {
  return { ...DEMO_SERVER_ENV, MAIDAN_PUBLIC_ORIGIN: origin ?? "https://try.maidan.world" };
}
