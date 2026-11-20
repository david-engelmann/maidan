import { DEMO_SERVER_ENV } from "./demo-config.js";
import { JEV_SERVER_ENV, jevEnabled } from "./demo-jev.js";

export const DEMO_PUBLIC_ORIGIN_VAR = "MAIDAN_PUBLIC_ORIGIN";

// The container's environment: the fixed demo settings plus the public origin.
// Nothing from the Worker's secrets is passed in, so the demo holds no
// production credential. With a TypeSafe key on the Worker, advice is turned
// on and pointed at the Worker itself (src/demo-jev.js): the key stays here.
export function demoEnv(origin, env = {}) {
  return {
    ...DEMO_SERVER_ENV,
    ...(jevEnabled(env) ? JEV_SERVER_ENV : {}),
    MAIDAN_PUBLIC_ORIGIN: origin ?? "https://try.maidan.world",
  };
}
