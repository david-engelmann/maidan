import { render, cleanup, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { SessionContext } from "../src/hooks/useSession.ts";
import { SessionStore } from "../src/core/session.ts";
import { Tools } from "../src/tools/Tools.tsx";
import { FetchRouter } from "./console-harness.tsx";

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

test("sign out is on screen while the credential mode is still loading", async () => {
  let releaseMe: (value: unknown) => void = () => {};
  const me = new Promise((resolve) => {
    releaseMe = resolve;
  });
  window.location.hash = "#/tools/prefs";
  const router = new FetchRouter([
    { match: /\/auth\/session$/, body: () => ({ member_id: "me", workspace_id: "ws", display_name: "Ada" }) },
    { match: /\/me$/, body: () => me },
    { match: /\/members\/me\/delivery-mode$/, body: () => ({ mode: "immediate" }) },
    { match: /\/members\/me\/email$/, body: () => ({ email: "ada@example.com" }) },
    { match: /\/members\/me\/notification-prefs$/, body: () => [] },
    { match: /\/members\/me\/channel-follows$/, body: () => [] },
    { match: /\/members\/me\/thread-follows$/, body: () => [] },
    { match: /\/workspaces\/ws\/channels$/, body: () => [] },
  ]);
  const store = new SessionStore("https://console.test", router.fn);
  globalThis.fetch = router.fn;
  const pending = store.refresh();
  render(
    <SessionContext.Provider value={store}>
      <Tools />
    </SessionContext.Provider>,
  );
  await waitFor(() => expect(screen.getByRole("button", { name: "Sign out" })).toBeTruthy());
  expect(screen.getByText("Checking…")).toBeTruthy();
  releaseMe({
    member_id: "me",
    workspace_id: "ws",
    is_bearer: false,
    capabilities: ["workspace:read"],
    known_capabilities: ["workspace:read", "thread:write"],
  });
  await pending;
  await waitFor(() => expect(screen.getByText("Session: your own sign-in, acting as you")).toBeTruthy());
  expect(screen.getByRole("button", { name: "Sign out" })).toBeTruthy();
});
