import { DEMO_SERVER_ENV } from "./demo-config.js";
import { adviceServerEnv } from "./demo-jev.js";

export const DEMO_PUBLIC_ORIGIN_VAR = "MAIDAN_PUBLIC_ORIGIN";

// The container's environment: the fixed demo settings plus the public origin.
// Nothing from the Worker's secrets is passed in, so the demo holds no
// production credential. With a key for a decision model on the Worker,
// advice is turned on and each such provider is pointed at the Worker itself
// (src/demo-jev.js): the keys stay here.
export function demoEnv(origin, env = {}) {
  return {
    ...DEMO_SERVER_ENV,
    ...adviceServerEnv(env),
    MAIDAN_PUBLIC_ORIGIN: origin ?? "https://try.maidan.world",
  };
}
