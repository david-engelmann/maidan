import { renderSignedIn, withSession } from "./console-harness.tsx";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test } from "vitest";
import { Artifacts } from "../src/artifacts/Artifacts.tsx";
import { SessionContext } from "../src/hooks/useSession.ts";
import { SessionStore } from "../src/core/session.ts";

afterEach(() => cleanup());

const SHA = "ab".repeat(32);

test("a task can be chosen, and a linked file shows its name and size", async () => {
  const user = userEvent.setup();
  const router = withSession([
    { match: /\/workspaces\/ws\/channels$/, body: () => [{ id: "c1", name: "general" }] },
    {
      match: /\/channels\/c1\/threads/,
      body: () => [
        { id: "t1", title: "Deploy v9", state: "open" },
        { id: "t2", title: "Open task", state: "open" },
      ],
    },
    { match: /\/threads\/t1\/artifacts$/, body: () => [{ sha256: SHA }] },
    { match: /\/threads\/t2\/artifacts$/, body: () => [] },
    { match: /\/artifacts\/[0-9a-f]+\/meta$/, body: () => ({ filename: "notes.txt", size_bytes: 18, mime_type: "text/plain" }) },
  ]);
  await renderSignedIn(<Artifacts />, router.fn);

  // The lookup field keeps the name the browser tests query.
  expect(screen.getByLabelText("sha256")).toBeTruthy();

  const evidence = screen.getByRole("region", { name: "A task's evidence" });
  const view = within(evidence);
  const select = await waitFor(() => view.getByRole("combobox", { name: "Choose a task" }));
  await waitFor(() => expect(view.getByRole("option", { name: "#general — Deploy v9" })).toBeTruthy());
  await user.selectOptions(select, "t1");

  const file = await waitFor(() => view.getByRole("button", { name: /notes\.txt/ }));
  expect(file.textContent).toContain("18 B");
  expect(file.textContent).toContain(`${SHA.slice(0, 12)}…`);
  expect(view.getByRole("link", { name: "Open task" }).getAttribute("href")).toBe("#/threads/t/t1");

  await user.click(file);
  expect((screen.getByLabelText("sha256") as HTMLInputElement).value).toBe(SHA);

  await user.clear(view.getByLabelText("Task id"));
  await user.type(view.getByLabelText("Task id"), "t2");
  await user.click(view.getByRole("button", { name: "Open" }));
  await waitFor(() => expect(view.getByText(/No files linked to this task yet/)).toBeTruthy());
});

test("a sha typed while the session is still signing in survives the sign-in", async () => {
  // The workspace arriving with the first sign-in once cleared the field and
  // the lookup, so a fast lookup showed nothing at all.
  const user = userEvent.setup();
  const router = withSession([{ match: /\/workspaces\/ws\/channels$/, body: () => [] }]);
  const store = new SessionStore("https://console.test", router.fn);
  globalThis.fetch = router.fn;
  render(
    <SessionContext.Provider value={store}>
      <Artifacts />
    </SessionContext.Provider>,
  );
  const field = screen.getByLabelText("sha256") as HTMLInputElement;
  await user.type(field, "0".repeat(64));
  await user.keyboard("{Enter}");
  await store.refresh();
  await waitFor(() => expect(screen.getByText("Not found, or not in this workspace.")).toBeTruthy());
  expect(field.value).toBe("0".repeat(64));
});
