import { render, cleanup, screen, fireEvent, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vitest";
import { SessionContext } from "../src/hooks/useSession.ts";
import { SessionStore } from "../src/core/session.ts";
import { THEME_KEY, setChoice } from "../src/core/theme.ts";
import { Tools } from "../src/tools/Tools.tsx";
import { FetchRouter } from "./console-harness.tsx";

beforeEach(() => {
  window.localStorage.clear();
  setChoice("dark");
  window.localStorage.clear();
});

afterEach(() => {
  cleanup();
  window.location.hash = "";
});

function renderDevice() {
  window.location.hash = "#/tools/device";
  const router = new FetchRouter([{ match: /\/auth\/session$/, status: 401, body: () => ({}) }]);
  globalThis.fetch = router.fn;
  const store = new SessionStore("https://console.test", router.fn);
  render(
    <SessionContext.Provider value={store}>
      <Tools />
    </SessionContext.Provider>,
  );
}

test("This device offers Dark, Light and System, with Dark chosen until someone picks", async () => {
  renderDevice();
  const group = await screen.findByRole("radiogroup", { name: "Theme" });
  const radios = screen.getAllByRole("radio");
  expect(radios.map((r) => r.textContent)).toEqual(["Dark", "Light", "System"]);
  expect(screen.getByRole("radio", { name: "Dark" }).getAttribute("aria-checked")).toBe("true");
  expect(group.querySelectorAll('[tabindex="0"]').length).toBe(1);
});

test("picking Light keeps it in this browser and switches the page", async () => {
  renderDevice();
  fireEvent.click(await screen.findByRole("radio", { name: "Light" }));
  await waitFor(() => expect(document.documentElement.dataset.theme).toBe("light"));
  expect(window.localStorage.getItem(THEME_KEY)).toBe("light");
  expect(screen.getByRole("radio", { name: "Light" }).getAttribute("aria-checked")).toBe("true");
});

test("the arrow keys move the choice, as in a native radio group", async () => {
  renderDevice();
  const group = await screen.findByRole("radiogroup", { name: "Theme" });
  fireEvent.keyDown(group, { key: "ArrowLeft" });
  await waitFor(() => expect(window.localStorage.getItem(THEME_KEY)).toBe("system"));
  expect(document.activeElement?.textContent).toBe("System");
  fireEvent.keyDown(group, { key: "ArrowRight" });
  await waitFor(() => expect(window.localStorage.getItem(THEME_KEY)).toBe("dark"));
  expect(document.documentElement.dataset.theme).toBe("dark");
});
